//! Static assets injected into transpiled documents: flex utilities, base CSS,
//! and third-party library references for QR/barcode rendering.

/// Where third-party JS libraries are loaded from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AssetsBase {
    /// Load libraries from a public CDN (jsDelivr).
    #[default]
    Cdn,
    /// Load libraries from a base URL/path serving the vendored assets, e.g.
    /// `/assets` or `file:///opt/lbl/assets`.
    Local(String),
}

impl AssetsBase {
    /// URL for the QR code library (exposes global `QRCode`).
    pub fn qrcode_url(&self) -> String {
        match self {
            AssetsBase::Cdn => "https://cdn.jsdelivr.net/npm/qrcode@1/build/qrcode.min.js".into(),
            AssetsBase::Local(base) => format!("{}/qrcode.min.js", base.trim_end_matches('/')),
        }
    }

    /// URL for the barcode library (exposes global `JsBarcode`).
    pub fn jsbarcode_url(&self) -> String {
        match self {
            AssetsBase::Cdn => {
                "https://cdn.jsdelivr.net/npm/jsbarcode@3/dist/JsBarcode.all.min.js".into()
            }
            AssetsBase::Local(base) => {
                format!("{}/JsBarcode.all.min.js", base.trim_end_matches('/'))
            }
        }
    }
}

/// Base + flex-utility CSS available to every transpiled label.
pub const BASE_CSS: &str = r#"
*,*::before,*::after{box-sizing:border-box}
html,body{margin:0;padding:0}
.lbl-label{display:flex;flex-direction:column}
.lbl-row{display:flex;flex-direction:row}
.lbl-col{display:flex;flex-direction:column}
.lbl-center{align-items:center;justify-content:center}
.lbl-between{justify-content:space-between}
.lbl-grow{flex:1 1 auto}
.lbl-wrap{flex-wrap:wrap}
.lbl-qr,.lbl-barcode{display:inline-flex;align-items:center;justify-content:center}
.lbl-qr canvas,.lbl-qr img,.lbl-barcode svg{max-width:100%;height:auto}
"#;

/// Additional CSS for preview mode: a neutral backdrop and a label boundary so
/// the document reads well in a browser/gallery.
pub const PREVIEW_CSS: &str = r#"
body{background:#e9e9ee;display:flex;align-items:center;justify-content:center;min-height:100vh}
.lbl-preview{background:#fff;box-shadow:0 1px 6px rgba(0,0,0,.25);outline:1px dashed #b9b9c4;outline-offset:4px}
"#;

/// JS that renders QR placeholders (`.lbl-qr[data-qr]`) into canvases.
///
/// Honors `window.__LBL_STYLE.qr.width` (pixels) when present, so the rendered
/// QR matches the configured physical size.
pub const QR_INIT_JS: &str = r#"
(function(){
  function render(){
    var st=(window.__LBL_STYLE&&window.__LBL_STYLE.qr)||{};
    document.querySelectorAll('.lbl-qr').forEach(function(el){
      if(el.dataset.rendered) return;
      var value = el.getAttribute('data-qr') || '';
      var canvas = document.createElement('canvas');
      el.appendChild(canvas);
      if(window.QRCode && QRCode.toCanvas){
        var opts={margin:0};
        if(st.width){opts.width=st.width;}
        QRCode.toCanvas(canvas, value, opts, function(){});
      }
      el.dataset.rendered = '1';
    });
  }
  if(document.readyState!=='loading'){render();}else{document.addEventListener('DOMContentLoaded',render);}
})();
"#;

/// JS that renders barcode placeholders (`.lbl-barcode[data-value]`).
///
/// Honors `window.__LBL_STYLE.barcode` (`width`/`height`/`fontSize`, all in
/// pixels) when present, so the rendered barcode matches the configured size.
pub const BARCODE_INIT_JS: &str = r#"
(function(){
  function render(){
    var st=(window.__LBL_STYLE&&window.__LBL_STYLE.barcode)||{};
    document.querySelectorAll('.lbl-barcode').forEach(function(el){
      if(el.dataset.rendered) return;
      var svg = document.createElementNS('http://www.w3.org/2000/svg','svg');
      el.appendChild(svg);
      if(window.JsBarcode){
        var opts={format: el.getAttribute('data-symbology')||'CODE128', margin:0};
        if(st.width){opts.width=st.width;}
        if(st.height){opts.height=st.height;}
        if(st.fontSize){opts.fontSize=st.fontSize;}
        try{ JsBarcode(svg, el.getAttribute('data-value')||'', opts); }catch(e){}
      }
      el.dataset.rendered = '1';
    });
  }
  if(document.readyState!=='loading'){render();}else{document.addEventListener('DOMContentLoaded',render);}
})();
"#;
