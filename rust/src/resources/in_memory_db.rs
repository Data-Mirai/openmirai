//! InMemoryDBResource — implements the `DBResource` trait with a purely
//! in-memory store.  Tables are modelled as `HashMap<String, Vec<Value>>`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::core::context::{DBResource, ResourceError};

// ---------------------------------------------------------------------------
// InMemoryDBResource
// ---------------------------------------------------------------------------

/// In-memory database resource for dev/testing.
///
/// Rows are stored per table name.  The `execute` method interprets a very
/// small subset of pseudo-SQL (INSERT / DELETE by `__table__` + `id`
/// convention) while also accepting raw JSON rows.  `fetch_one` and
/// `fetch_all` similarly use the `__table__` convention.
#[derive(Debug)]
pub struct InMemoryDBResource {
    tables: Arc<RwLock<HashMap<String, HashMap<String, Value>>>>,
}

impl InMemoryDBResource {
    pub fn new() -> Self {
        Self {
            tables: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryDBResource {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DBResource for InMemoryDBResource {
    async fn execute(
        &self,
        query: &str,
        params: &[Value],
    ) -> Result<Value, ResourceError> {
        // Convention: first param may be a JSON object with `__table__` key.
        if let Some(obj) = params.first().and_then(|p| p.as_object()) {
            let table = obj
                .get("__table__")
                .and_then(|v| v.as_str())
                .unwrap_or("_default")
                .to_string();
            let row_id = obj.get("id").and_then(|v| v.as_str()).map(String::from);

            let mut tables = self.tables.write().await;
            let tbl = tables.entry(table).or_default();

            let query_upper = query.to_uppercase();
            if query_upper.contains("DELETE") {
                if let Some(id) = row_id {
                    tbl.remove(&id);
                }
            } else if let Some(id) = row_id {
                // Upsert: store the full object minus __table__.
                let mut row = obj.clone();
                row.remove("__table__");
                tbl.insert(id, Value::Object(row));
            }
        }

        Ok(Value::Null)
    }

    async fn fetch_one(
        &self,
        _query: &str,
        params: &[Value],
    ) -> Result<Option<Value>, ResourceError> {
        if let Some(obj) = params.first().and_then(|p| p.as_object()) {
            let table = obj
                .get("__table__")
                .and_then(|v| v.as_str())
                .unwrap_or("_default");
            let row_id = obj.get("id").and_then(|v| v.as_str());

            let tables = self.tables.read().await;
            if let (Some(tbl), Some(id)) = (tables.get(table), row_id) {
                return Ok(tbl.get(id).cloned());
            }
        }
        Ok(None)
    }

    async fn fetch_all(
        &self,
        _query: &str,
        params: &[Value],
    ) -> Result<Vec<Value>, ResourceError> {
        if let Some(obj) = params.first().and_then(|p| p.as_object()) {
            let table = obj
                .get("__table__")
                .and_then(|v| v.as_str())
                .unwrap_or("_default");

            let tables = self.tables.read().await;
            if let Some(tbl) = tables.get(table) {
                return Ok(tbl.values().cloned().collect());
            }
        }
        Ok(vec![])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn insert_and_fetch_one() {
        let db = InMemoryDBResource::new();

        let params = vec![json!({
            "__table__": "users",
            "id": "u1",
            "name": "Alice"
        })];

        db.execute("INSERT", &params).await.unwrap();

        let fetch_params = vec![json!({"__table__": "users", "id": "u1"})];
        let row = db.fetch_one("SELECT", &fetch_params).await.unwrap();
        assert!(row.is_some());
        let row = row.unwrap();
        assert_eq!(row["name"], "Alice");
        assert_eq!(row["id"], "u1");
        // __table__ should be stripped from stored value.
        assert!(row.get("__table__").is_none());
    }

    #[tokio::test]
    async fn fetch_one_missing() {
        let db = InMemoryDBResource::new();
        let params = vec![json!({"__table__": "users", "id": "nope"})];
        let row = db.fetch_one("SELECT", &params).await.unwrap();
        assert!(row.is_none());
    }

    #[tokio::test]
    async fn fetch_all() {
        let db = InMemoryDBResource::new();

        for i in 0..3 {
            let params = vec![json!({
                "__table__": "items",
                "id": format!("i{}", i),
                "value": i
            })];
            db.execute("INSERT", &params).await.unwrap();
        }

        let params = vec![json!({"__table__": "items"})];
        let rows = db.fetch_all("SELECT", &params).await.unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[tokio::test]
    async fn fetch_all_empty_table() {
        let db = InMemoryDBResource::new();
        let params = vec![json!({"__table__": "empty"})];
        let rows = db.fetch_all("SELECT", &params).await.unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn delete_row() {
        let db = InMemoryDBResource::new();

        let params = vec![json!({"__table__": "t", "id": "r1", "data": "x"})];
        db.execute("INSERT", &params).await.unwrap();

        let del_params = vec![json!({"__table__": "t", "id": "r1"})];
        db.execute("DELETE", &del_params).await.unwrap();

        let fetch_params = vec![json!({"__table__": "t", "id": "r1"})];
        let row = db.fetch_one("SELECT", &fetch_params).await.unwrap();
        assert!(row.is_none());
    }

    #[tokio::test]
    async fn upsert_overwrites() {
        let db = InMemoryDBResource::new();

        let params1 = vec![json!({"__table__": "t", "id": "r1", "val": 1})];
        db.execute("INSERT", &params1).await.unwrap();

        let params2 = vec![json!({"__table__": "t", "id": "r1", "val": 99})];
        db.execute("INSERT", &params2).await.unwrap();

        let fetch = vec![json!({"__table__": "t", "id": "r1"})];
        let row = db.fetch_one("SELECT", &fetch).await.unwrap().unwrap();
        assert_eq!(row["val"], 99);
    }

    #[tokio::test]
    async fn empty_params() {
        let db = InMemoryDBResource::new();
        let result = db.execute("NOOP", &[]).await.unwrap();
        assert_eq!(result, Value::Null);

        let row = db.fetch_one("NOOP", &[]).await.unwrap();
        assert!(row.is_none());

        let rows = db.fetch_all("NOOP", &[]).await.unwrap();
        assert!(rows.is_empty());
    }
}
