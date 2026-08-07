//! SqliteDBResource -- persistent DBResource backed by SQLite.
//!
//! Uses WAL mode for concurrent reads (REGLA-504).
//! Wraps synchronous rusqlite calls in `tokio::task::spawn_blocking`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::core::context::{DBResource, ResourceError};

/// Persistent database resource backed by SQLite.
pub struct SqliteDBResource {
    conn: Arc<Mutex<Connection>>,
}

// rusqlite::Connection is Send but not Sync. Mutex makes it Send + Sync.
unsafe impl Send for SqliteDBResource {}
unsafe impl Sync for SqliteDBResource {}

impl SqliteDBResource {
    /// Open (or create) a SQLite database at `path`.
    ///
    /// Enables WAL mode and foreign keys by default.
    pub fn new(path: &str) -> Result<Self, ResourceError> {
        let conn = Connection::open(path)
            .map_err(|e| ResourceError::Database(format!("failed to open SQLite: {e}")))?;

        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA busy_timeout=10000; PRAGMA foreign_keys=ON;",
        )
        .map_err(|e| ResourceError::Database(format!("failed to set PRAGMAs: {e}")))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create an in-memory SQLite database (for testing).
    pub fn new_in_memory() -> Result<Self, ResourceError> {
        Self::new(":memory:")
    }
}

/// Convert a serde_json::Value to a rusqlite-compatible param.
fn value_to_rusqlite(v: &Value) -> Box<dyn rusqlite::types::ToSql> {
    match v {
        Value::Null => Box::new(rusqlite::types::Null),
        Value::Bool(b) => Box::new(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Box::new(i)
            } else if let Some(f) = n.as_f64() {
                Box::new(f)
            } else {
                Box::new(n.to_string())
            }
        }
        Value::String(s) => Box::new(s.clone()),
        _ => Box::new(v.to_string()),
    }
}

/// Convert a rusqlite Row to a JSON object using column names.
fn row_to_json(row: &rusqlite::Row<'_>, col_count: usize, col_names: &[String]) -> Value {
    let mut obj = serde_json::Map::new();
    for (i, col_name) in col_names.iter().enumerate().take(col_count) {
        let val: Value = match row.get_ref(i) {
            Ok(rusqlite::types::ValueRef::Null) => Value::Null,
            Ok(rusqlite::types::ValueRef::Integer(n)) => json!(n),
            Ok(rusqlite::types::ValueRef::Real(f)) => json!(f),
            Ok(rusqlite::types::ValueRef::Text(s)) => {
                let s = String::from_utf8_lossy(s).to_string();
                // Try to parse as JSON if it looks like it
                if (s.starts_with('{') && s.ends_with('}'))
                    || (s.starts_with('[') && s.ends_with(']'))
                {
                    serde_json::from_str(&s).unwrap_or(Value::String(s))
                } else {
                    Value::String(s)
                }
            }
            Ok(rusqlite::types::ValueRef::Blob(b)) => {
                json!(format!("<blob:{} bytes>", b.len()))
            }
            Err(_) => Value::Null,
        };
        obj.insert(col_name.clone(), val);
    }
    Value::Object(obj)
}

#[async_trait]
impl DBResource for SqliteDBResource {
    async fn execute(&self, query: &str, params: &[Value]) -> Result<Value, ResourceError> {
        let conn = Arc::clone(&self.conn);
        let query = query.to_string();
        let params: Vec<Value> = params.to_vec();

        tracing::debug!(query = %query, params_len = params.len(), "SQLite: executing update query");

        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ResourceError::Database(format!("mutex poisoned: {e}")))?;

            let param_refs: Vec<Box<dyn rusqlite::types::ToSql>> =
                params.iter().map(value_to_rusqlite).collect();
            let param_slice: Vec<&dyn rusqlite::types::ToSql> =
                param_refs.iter().map(|b| b.as_ref()).collect();

            let rows_affected = conn
                .execute(&query, param_slice.as_slice())
                .map_err(|e| {
                    tracing::error!(query = %query, error = %e, "SQLite update execution failed");
                    ResourceError::Database(format!("execute failed: {e}"))
                })?;

            tracing::debug!(query = %query, rows_affected = rows_affected, "SQLite update executed successfully");
            Ok(json!({ "rows_affected": rows_affected }))
        })
        .await
        .map_err(|e| ResourceError::Database(format!("spawn_blocking join: {e}")))?
    }

    async fn fetch_one(
        &self,
        query: &str,
        params: &[Value],
    ) -> Result<Option<Value>, ResourceError> {
        let conn = Arc::clone(&self.conn);
        let query = query.to_string();
        let params: Vec<Value> = params.to_vec();

        tracing::debug!(query = %query, params_len = params.len(), "SQLite: fetching one row");

        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ResourceError::Database(format!("mutex poisoned: {e}")))?;

            let param_refs: Vec<Box<dyn rusqlite::types::ToSql>> =
                params.iter().map(value_to_rusqlite).collect();
            let param_slice: Vec<&dyn rusqlite::types::ToSql> =
                param_refs.iter().map(|b| b.as_ref()).collect();

            let mut stmt = conn
                .prepare(&query)
                .map_err(|e| {
                    tracing::error!(query = %query, error = %e, "SQLite prepare statement failed in fetch_one");
                    ResourceError::Database(format!("prepare failed: {e}"))
                })?;

            let col_count = stmt.column_count();
            let col_names: Vec<String> = (0..col_count)
                .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
                .collect();

            let mut rows = stmt
                .query(param_slice.as_slice())
                .map_err(|e| {
                    tracing::error!(query = %query, error = %e, "SQLite query failed in fetch_one");
                    ResourceError::Database(format!("query failed: {e}"))
                })?;

            match rows
                .next()
                .map_err(|e| ResourceError::Database(format!("next failed: {e}")))?
            {
                Some(row) => Ok(Some(row_to_json(row, col_count, &col_names))),
                None => Ok(None),
            }
        })
        .await
        .map_err(|e| ResourceError::Database(format!("spawn_blocking join: {e}")))?
    }

    async fn fetch_all(&self, query: &str, params: &[Value]) -> Result<Vec<Value>, ResourceError> {
        let conn = Arc::clone(&self.conn);
        let query = query.to_string();
        let params: Vec<Value> = params.to_vec();

        tracing::debug!(query = %query, params_len = params.len(), "SQLite: fetching all rows");

        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| ResourceError::Database(format!("mutex poisoned: {e}")))?;

            let param_refs: Vec<Box<dyn rusqlite::types::ToSql>> =
                params.iter().map(value_to_rusqlite).collect();
            let param_slice: Vec<&dyn rusqlite::types::ToSql> =
                param_refs.iter().map(|b| b.as_ref()).collect();

            let mut stmt = conn
                .prepare(&query)
                .map_err(|e| {
                    tracing::error!(query = %query, error = %e, "SQLite prepare statement failed in fetch_all");
                    ResourceError::Database(format!("prepare failed: {e}"))
                })?;

            let col_count = stmt.column_count();
            let col_names: Vec<String> = (0..col_count)
                .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
                .collect();

            let mut rows = stmt
                .query(param_slice.as_slice())
                .map_err(|e| {
                    tracing::error!(query = %query, error = %e, "SQLite query failed in fetch_all");
                    ResourceError::Database(format!("query failed: {e}"))
                })?;

            let mut results = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|e| ResourceError::Database(format!("next failed: {e}")))?
            {
                results.push(row_to_json(row, col_count, &col_names));
            }
            Ok(results)
        })
        .await
        .map_err(|e| ResourceError::Database(format!("spawn_blocking join: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn execute_and_fetch() {
        let db = SqliteDBResource::new_in_memory().unwrap();

        db.execute(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT, value REAL)",
            &[],
        )
        .await
        .unwrap();

        db.execute(
            "INSERT INTO items (name, value) VALUES (?1, ?2)",
            &[json!("alpha"), json!(1.5)],
        )
        .await
        .unwrap();

        db.execute(
            "INSERT INTO items (name, value) VALUES (?1, ?2)",
            &[json!("beta"), json!(2.5)],
        )
        .await
        .unwrap();

        let one = db
            .fetch_one("SELECT * FROM items WHERE name = ?1", &[json!("alpha")])
            .await
            .unwrap();
        assert!(one.is_some());
        let row = one.unwrap();
        assert_eq!(row["name"], json!("alpha"));
        assert_eq!(row["value"], json!(1.5));

        let all = db
            .fetch_all("SELECT * FROM items ORDER BY name", &[])
            .await
            .unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0]["name"], json!("alpha"));
        assert_eq!(all[1]["name"], json!("beta"));
    }

    #[tokio::test]
    async fn fetch_one_returns_none_for_no_match() {
        let db = SqliteDBResource::new_in_memory().unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)", &[])
            .await
            .unwrap();
        let result = db
            .fetch_one("SELECT * FROM t WHERE id = 999", &[])
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn execute_returns_rows_affected() {
        let db = SqliteDBResource::new_in_memory().unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT)", &[])
            .await
            .unwrap();
        db.execute("INSERT INTO t (val) VALUES (?1)", &[json!("a")])
            .await
            .unwrap();
        db.execute("INSERT INTO t (val) VALUES (?1)", &[json!("b")])
            .await
            .unwrap();

        let result = db.execute("DELETE FROM t", &[]).await.unwrap();
        assert_eq!(result["rows_affected"], json!(2));
    }
}
