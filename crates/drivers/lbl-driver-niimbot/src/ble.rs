//! Browser BLE transport constants and B1 connect framing for NIIMBOT.
//!
//! Web Bluetooth GATT UUIDs, paced-write parameters, and the post-connect
//! handshake packets are protocol facts — they belong with the driver, not in
//! a particular UI. Consumers (WASM, host tools) apply them over their own
//! transport.

/// NIIMBOT BLE GATT service UUID (appears after connect; often not advertised).
pub const BLE_SERVICE_UUID: &str = "e7810a71-73ae-499d-8c15-faa9aef0c3f2";

/// NIIMBOT BLE GATT characteristic UUID (write + notify).
pub const BLE_CHARACTERISTIC_UUID: &str = "bef8d6c9-9c21-4c9e-b632-bd58c1009f9f";

/// Gap between unacked BLE writes for the B1 task (~10 ms).
pub const B1_PACE_MS: u32 = 10;

/// Max bytes per bundled BLE write for the B1 task.
pub const B1_BUNDLE_MAX: usize = 240;

const HEAD: [u8; 2] = [0x55, 0x55];
const TAIL: [u8; 2] = [0xAA, 0xAA];
const PRINTER_STATUS_DATA: u8 = 0xa5;
const PRINTER_INFO: u8 = 0x40;
const HEARTBEAT: u8 = 0xdc;

/// Initial BLE connect packet (raw, with leading `0x03` prefix).
pub fn b1_ble_connect_packet() -> Vec<u8> {
    vec![0x03, 0x55, 0x55, 0xc1, 0x01, 0x01, 0xc1, 0xaa, 0xaa]
}

const B1_INFO_SUBCODES: &[u8] = &[0x08, 0x0b, 0x0d, 0x0a, 0x07, 0x03, 0x0c, 0x09];

/// Frame a NIIMBOT command packet (`55 55 cmd len data checksum aa aa`).
pub fn frame_packet(command: u8, data: &[u8]) -> Vec<u8> {
    let mut checksum = command ^ (data.len() as u8);
    for &b in data {
        checksum ^= b;
    }
    let mut out = Vec::with_capacity(2 + 1 + 1 + data.len() + 1 + 2);
    out.extend_from_slice(&HEAD);
    out.push(command);
    out.push(data.len() as u8);
    out.extend_from_slice(data);
    out.push(checksum);
    out.extend_from_slice(&TAIL);
    out
}

/// Post-connect B1 session handshake packets (status + info probes + heartbeat).
pub fn b1_handshake_packets() -> Vec<Vec<u8>> {
    let mut out = vec![frame_packet(PRINTER_STATUS_DATA, &[0x01])];
    for &sub in B1_INFO_SUBCODES {
        out.push(frame_packet(PRINTER_INFO, &[sub]));
    }
    out.push(frame_packet(HEARTBEAT, &[0x04]));
    out
}

/// Split a NIIMBOT job stream into framed packets for BLE writes.
pub fn split_framed_packets(data: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= data.len() {
        if data[i] != 0x55 || data[i + 1] != 0x55 {
            i += 1;
            continue;
        }
        let len = data[i + 3] as usize;
        let end = i + 4 + len + 3;
        if end > data.len() {
            break;
        }
        if data[end - 2] == 0xaa && data[end - 1] == 0xaa {
            out.push(data[i..end].to_vec());
        }
        i = end;
    }
    if out.is_empty() && !data.is_empty() {
        out.push(data.to_vec());
    }
    out
}

/// Bundle framed packets into fewer BLE writes (B1 throughput optimization).
pub fn bundle_framed_packets(frames: &[Vec<u8>], max_bytes: usize) -> Vec<Vec<u8>> {
    let mut bundles = Vec::new();
    let mut current: Vec<u8> = Vec::new();
    for frame in frames {
        if !current.is_empty() && current.len() + frame.len() > max_bytes {
            bundles.push(std::mem::take(&mut current));
        }
        if frame.len() > max_bytes && current.is_empty() {
            bundles.push(frame.clone());
            continue;
        }
        current.extend_from_slice(frame);
    }
    if !current.is_empty() {
        bundles.push(current);
    }
    bundles
}

/// Extract the payload for `response_cmd` from a notify buffer that may contain
/// multiple `55 55 … aa aa` frames. Returns `(payload, consumed_through)` when
/// found.
pub fn packet_payload_for_command(buffer: &[u8], response_cmd: u8) -> Option<(Vec<u8>, usize)> {
    let mut i = 0;
    while i + 4 <= buffer.len() {
        if buffer[i] != 0x55 || buffer[i + 1] != 0x55 {
            i += 1;
            continue;
        }
        let cmd = buffer[i + 2];
        let len = buffer[i + 3] as usize;
        let end = i + 4 + len + 3;
        if end > buffer.len() {
            return None;
        }
        if buffer[end - 2] != 0xaa || buffer[end - 1] != 0xaa {
            i += 1;
            continue;
        }
        if cmd == response_cmd {
            let payload = buffer[i + 4..i + 4 + len].to_vec();
            return Some((payload, end));
        }
        i = end;
    }
    None
}

/// Response command byte for a `PrinterInfo(ModelId)` reply.
pub const PRINTER_MODEL_ID_RESPONSE: u8 = 0x48;

/// Extract a ModelId payload from a BLE notify chunk, if present.
pub fn model_id_payload_from_notify(buffer: &[u8]) -> Option<Vec<u8>> {
    packet_payload_for_command(buffer, PRINTER_MODEL_ID_RESPONSE).map(|(p, _)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_split() {
        let a = frame_packet(0x01, &[0x01]);
        let b = frame_packet(0x40, &[0x08]);
        let mut job = a.clone();
        job.extend_from_slice(&b);
        let frames = split_framed_packets(&job);
        assert_eq!(frames, vec![a, b]);
    }

    #[test]
    fn extracts_payload_for_command() {
        let pkt = frame_packet(0x1b, &[0x11, 0x22]);
        let (payload, end) = packet_payload_for_command(&pkt, 0x1b).unwrap();
        assert_eq!(payload, vec![0x11, 0x22]);
        assert_eq!(end, pkt.len());
    }

    #[test]
    fn extracts_model_id_payload() {
        let pkt = frame_packet(PRINTER_MODEL_ID_RESPONSE, &[0x09]);
        assert_eq!(model_id_payload_from_notify(&pkt), Some(vec![0x09]));
    }

    #[test]
    fn b1_handshake_nonempty() {
        assert!(!b1_handshake_packets().is_empty());
        assert_eq!(b1_ble_connect_packet()[0], 0x03);
    }
}
