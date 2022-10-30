use aws_config::meta::region::RegionProviderChain;
use aws_sdk_secretsmanager::{output::GetSecretValueOutput, Client};
use serde_json::Value;
use tracing::info;

async fn bench<T>(
    message: impl Into<&str> + Send,
    target_fn: impl std::future::Future<Output = T> + Send,
) -> T {
    let now = std::time::Instant::now();
    let r = target_fn.await;
    let duration_time = format!("{:?}", now.elapsed());
    let message = message.into();
    info!(duration_time, message);
    r
}

async fn load_url() -> Option<String> {
    let region_provider = RegionProviderChain::default_provider().or_else("ap-northeast-3");

    let shared_config = bench("load shared config from env", async {
        aws_config::from_env().region(region_provider).load().await
    })
    .await;
    let client = bench("construct client", async { Client::new(&shared_config) }).await;

    let get_secret_value = client.get_secret_value();
    let secret_id = get_secret_value.secret_id("SecretsManager-02");

    let sent = bench("send", async { secret_id.send().await }).await;
    let resp = sent.unwrap_or(GetSecretValueOutput::builder().build());

    let value = resp.secret_string();

    let secret_info: Option<Value> = if let Some(value) = value {
        serde_json::from_str(value).ok()
    } else {
        None
    };

    let url = if let Some(secret_info) = secret_info {
        let host: &str = &secret_info["host_proxy"].as_str().unwrap_or("localhost");
        let username: &str = &secret_info["username"].as_str().unwrap_or("root");
        let password: &str = &secret_info["password"].as_str().unwrap_or("password");
        let database: &str = &secret_info["dbname"].as_str().unwrap_or("test_db");

        format!("mysql://{}:{}@{}/{}", username, password, host, database)
    } else {
        let host: &str = "localhost";
        let username: &str = "root";
        let password: &str = "password";
        let database: &str = "test_db";

        format!("mysql://{}:{}@{}/{}", username, password, host, database)
    };

    Some(url)
}

async fn establish_connection() -> Option<sqlx::MySqlPool> {
    let url = load_url().await?;

    let c = async {
        sqlx::mysql::MySqlPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .unwrap()
    };

    let pool = bench("establish connection", c).await;
    Some(pool)
}
pub async fn establish_connection_or_get_cache() -> &'static sqlx::MySqlPool {
    static mut POOL: Option<sqlx::MySqlPool> = None;
    unsafe {
        if POOL.is_none() {
            POOL = establish_connection().await;
        }
    }
    let result = match unsafe { POOL.as_ref() } {
        None => todo!(),
        Some(pool) => pool,
    };
    result
}
