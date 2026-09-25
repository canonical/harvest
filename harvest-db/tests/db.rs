use harvest_db::test_support::TestDb;
use serde_json::json;

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn migrates_and_round_trips_typed_parameters() {
    let t = TestDb::new().await;
    t.db.execute(
        "INSERT INTO users (id, email, name, provider, role, created_at)
         VALUES ($id, $email, $name, 'local', 'admin', $now)",
        json!({ "id": "u1", "email": "a@b.c", "name": "Ann", "now": "2026-01-02T03:04:05.123456+00:00" }),
    ).await.unwrap();
    let rows = t.db.query(
        "SELECT id, created_at, $n::int AS n, ARRAY['x', 'y'] AS arr,
                json_build_object('k', 1) AS obj, id = ANY($ids) AS listed
         FROM users WHERE email = $email",
        json!({ "email": "a@b.c", "n": 7, "ids": ["u1", "u2"] }),
    ).await.unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row["id"], "u1");
    assert_eq!(row["created_at"], "2026-01-02T03:04:05.123456+00:00");
    assert_eq!(row["n"], 7);
    assert_eq!(row["arr"], json!(["x", "y"]));
    assert_eq!(row["obj"], json!({ "k": 1 }));
    assert_eq!(row["listed"], true);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn dropped_transaction_rolls_back() {
    let t = TestDb::new().await;
    {
        let tx = t.db.begin().await.unwrap();
        tx.execute("INSERT INTO app_flags (key) VALUES ($k)", json!({ "k": "x" })).await.unwrap();
    }
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let rows = t.db.query("SELECT key FROM app_flags", json!({})).await.unwrap();
    assert!(rows.is_empty());
}
