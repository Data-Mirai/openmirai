// Example implementation of a custom OpenMirai resource adapter.
// This is not compiled with the main crate, but serves as a copy-paste template.

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

use openmirai_engine::core::context::{DBResource, ResourceError};
use openmirai_engine::core::schema::TableSchema;

pub struct InMemoryDb {
    tables: std::sync::Mutex<HashMap<String, Vec<HashMap<String, Value>>>>,
}

impl InMemoryDb {
    pub fn new() -> Self {
        Self {
            tables: std::sync::Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl DBResource for InMemoryDb {
    async fn execute(&self, query: &str, params: &[Value]) -> Result<Vec<HashMap<String, Value>>, ResourceError> {
        // Simple in-memory stub that simulates execution or returns mock records.
        println!("Executing InMemory query: {} with params {:?}", query, params);
        
        let mut results = Vec::new();
        let mut row = HashMap::new();
        row.insert("id".to_string(), Value::from(1));
        row.insert("query_echo".to_string(), Value::from(query));
        results.push(row);
        
        Ok(results)
    }

    async fn create_table(&self, name: &str, schema: &TableSchema) -> Result<(), ResourceError> {
        println!("Creating InMemory table {} with schema {:?}", name, schema);
        Ok(())
    }

    async fn list_tables(&self) -> Result<Vec<String>, ResourceError> {
        let tables = self.tables.lock().unwrap();
        Ok(tables.keys().cloned().collect())
    }

    async fn query_table(
        &self,
        table: &str,
        columns: &[&str],
        filter: &str,
        limit: usize,
    ) -> Result<Vec<HashMap<String, Value>>, ResourceError> {
        println!("Querying table {} (cols: {:?}, filter: {}, limit: {})", table, columns, filter, limit);
        Ok(vec![])
    }

    async fn upsert_row(&self, table: &str, row: HashMap<String, Value>) -> Result<(), ResourceError> {
        println!("Upserting row into table {}: {:?}", table, row);
        Ok(())
    }

    async fn delete_row(&self, table: &str, primary_key: &Value) -> Result<(), ResourceError> {
        println!("Deleting row from table {} with PK {:?}", table, primary_key);
        Ok(())
    }
}
