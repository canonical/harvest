use anyhow::Result;
use neo4rs::{query, Graph, Row};
use serde_json::Value;

pub struct Neo4jClient {
    graph: Graph,
}

impl Neo4jClient {
    pub async fn new(uri: &str, user: &str, password: &str) -> Result<Self> {
        let graph = Graph::new(uri, user, password).await?;
        Ok(Self { graph })
    }

    pub async fn run(&self, cypher: &str) -> Result<()> {
        self.graph.run(query(cypher)).await?;
        Ok(())
    }

    pub async fn query_read(&self, cypher: &str, params: Value) -> Result<Vec<Value>> {
        let mut q = query(cypher);

        if let Value::Object(map) = params {
            for (key, val) in map {
                q = match val {
                    Value::String(s)  => q.param(&key, s),
                    Value::Number(n) if n.is_i64() => q.param(&key, n.as_i64().unwrap()),
                    Value::Number(n)  => q.param(&key, n.as_f64().unwrap()),
                    Value::Bool(b)    => q.param(&key, b),
                    Value::Null       => q,
                    other             => q.param(&key, other.to_string()),
                };
            }
        }

        let mut result = self.graph.execute(q).await?;
        let mut rows = Vec::new();
        while let Some(row) = result.next().await? {
            rows.push(row_to_json(&row));
        }
        Ok(rows)
    }
}

fn row_to_json(row: &Row) -> Value {
    row.to::<Value>().unwrap_or(Value::Null)
}
