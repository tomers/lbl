//! Batch label selection: filter, skip, take, and index picking.

use serde::Deserialize;
use serde_json::Value;

use crate::TemplateError;

/// Which labels to render from a batch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct BatchSelection {
    /// Case-insensitive substring match against any value in the record.
    #[serde(default)]
    pub filter: Option<String>,
    /// Skip this many labels after prior selection steps.
    #[serde(default)]
    pub skip: usize,
    /// Keep at most this many labels.
    #[serde(default)]
    pub take: Option<usize>,
    /// Keep only the last label after prior selection steps.
    #[serde(default)]
    pub last: bool,
    /// Restrict to these zero-based batch indices before filter/skip/take.
    #[serde(default)]
    pub indices: Option<Vec<usize>>,
}

impl BatchSelection {
    /// Convenience for `--first` / `--take 1`.
    pub fn first() -> Self {
        Self {
            take: Some(1),
            ..Self::default()
        }
    }

    /// Convenience for `--last`.
    pub fn last() -> Self {
        Self {
            last: true,
            ..Self::default()
        }
    }
}

/// Flatten a JSON value into searchable strings (matches the preview UI filter).
pub fn flatten_values(value: &Value) -> Vec<String> {
    match value {
        Value::Null => vec![],
        Value::Bool(b) => vec![b.to_string()],
        Value::Number(n) => vec![n.to_string()],
        Value::String(s) => vec![s.clone()],
        Value::Array(items) => items.iter().flat_map(flatten_values).collect(),
        Value::Object(map) => map.values().flat_map(flatten_values).collect(),
    }
}

/// Whether `record` matches a case-insensitive substring query.
pub fn record_matches_query(record: &Value, query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    let q = q.to_lowercase();
    flatten_values(record)
        .iter()
        .any(|value| value.to_lowercase().contains(&q))
}

/// Apply selection steps and return zero-based batch indices to render, in order.
pub fn select_batch_indices(
    records: &[Value],
    selection: &BatchSelection,
) -> Result<Vec<usize>, TemplateError> {
    let mut indices: Vec<usize> = match &selection.indices {
        Some(want) if !want.is_empty() => want.clone(),
        _ => (0..records.len()).collect(),
    };

    indices.retain(|&i| i < records.len());

    if let Some(query) = &selection.filter {
        if !query.trim().is_empty() {
            indices.retain(|&i| record_matches_query(&records[i], query));
        }
    }

    if selection.skip > 0 {
        if selection.skip >= indices.len() {
            indices.clear();
        } else {
            indices.drain(..selection.skip);
        }
    }

    if selection.last {
        if let Some(&last) = indices.last() {
            indices = vec![last];
        }
    } else if let Some(take) = selection.take {
        indices.truncate(take);
    }

    if indices.is_empty() {
        return Err(TemplateError::Data(
            "batch selection matched no labels".into(),
        ));
    }

    Ok(indices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sopranos() -> Vec<Value> {
        vec![
            json!({"name": "Tony Soprano", "title": "Boss"}),
            json!({"name": "Carmela Soprano", "title": "Homemaker"}),
            json!({"name": "Christopher Moltisanti", "title": "Soldier"}),
        ]
    }

    #[test]
    fn filter_matches_substring() {
        let records = sopranos();
        let sel = BatchSelection {
            filter: Some("car".into()),
            ..Default::default()
        };
        assert_eq!(select_batch_indices(&records, &sel).unwrap(), vec![1]);
    }

    #[test]
    fn first_takes_first_after_filter() {
        let records = sopranos();
        let sel = BatchSelection {
            filter: Some("soprano".into()),
            take: Some(1),
            ..Default::default()
        };
        assert_eq!(select_batch_indices(&records, &sel).unwrap(), vec![0]);
    }

    #[test]
    fn last_takes_last_after_filter() {
        let records = sopranos();
        let sel = BatchSelection {
            filter: Some("soprano".into()),
            last: true,
            ..Default::default()
        };
        assert_eq!(select_batch_indices(&records, &sel).unwrap(), vec![1]);
    }

    #[test]
    fn last_after_skip() {
        let records = sopranos();
        let sel = BatchSelection {
            skip: 1,
            last: true,
            ..Default::default()
        };
        assert_eq!(select_batch_indices(&records, &sel).unwrap(), vec![2]);
    }

    #[test]
    fn skip_and_take() {
        let records = sopranos();
        let sel = BatchSelection {
            skip: 1,
            take: Some(1),
            ..Default::default()
        };
        assert_eq!(select_batch_indices(&records, &sel).unwrap(), vec![1]);
    }

    #[test]
    fn explicit_indices() {
        let records = sopranos();
        let sel = BatchSelection {
            indices: Some(vec![0, 2]),
            ..Default::default()
        };
        assert_eq!(select_batch_indices(&records, &sel).unwrap(), vec![0, 2]);
    }

    #[test]
    fn empty_selection_errors() {
        let records = sopranos();
        let sel = BatchSelection {
            filter: Some("zzz".into()),
            ..Default::default()
        };
        assert!(select_batch_indices(&records, &sel).is_err());
    }

    #[test]
    fn deserializes_indices_from_json() {
        let sel: BatchSelection =
            serde_json::from_value(json!({"indices": [0, 2], "skip": 0})).unwrap();
        assert_eq!(sel.indices, Some(vec![0, 2]));
        assert_eq!(sel.skip, 0);
        assert!(!sel.last);
        assert!(sel.filter.is_none());
        assert!(sel.take.is_none());
    }

    #[test]
    fn deserializes_empty_object_as_default() {
        let sel: BatchSelection = serde_json::from_value(json!({})).unwrap();
        assert_eq!(sel, BatchSelection::default());
    }
}
