use dotenv::dotenv;
use std::env;

use aws_config::meta::region::RegionProviderChain;
use aws_sdk_secretsmanager::{output::GetSecretValueOutput, Client};
use serde_json::Value;
use thiserror::Error;

use run_test_async::bench as do_with_bench;

#[derive(Debug, Error)]
enum Error {
    #[error("NotSatisfiedSecretInfo")]
    NotSatisfiedSecretInfo,
    #[error("NotSatisfiedUrlFromEnv")]
    NotSatisfiedUrlFromEnv,
    #[error("FailureGetSecretString")]
    FailureGetSecretString,
}

fn make_url_from_value(map: Value) -> anyhow::Result<String> {
    if let (Some(username), Some(password), Some(host), Some(database)) = (
        map["username"].as_str(),
        map["password"].as_str(),
        map["host"].as_str(),
        map["database"].as_str(),
    ) {
        return Ok(format!(
            "mysql://{}:{}@{}/{}",
            username, password, host, database
        ));
    } else {
        return Err(Error::NotSatisfiedSecretInfo.into());
    }
}
fn make_url_from_env() -> anyhow::Result<String> {
    dotenv().ok();
    env::var("DATABASE_URL")
        .map(|str| str.to_string())
        .or_else(|_| Err(Error::NotSatisfiedUrlFromEnv.into()))
}
fn make_url_default_local() -> anyhow::Result<String> {
    let username: &str = "root";
    let password: &str = "password";
    let host: &str = "localhost";
    let database: &str = "test_db";
    return Ok(format!(
        "mysql://{}:{}@{}/{}",
        username, password, host, database
    ));
}

async fn construct_url_for_aws<'a, F>(
    getter_region_and_secrets_manager: F,
) -> anyhow::Result<String>
where
    F: Fn() -> Option<(&'static str, &'a str)>,
{
    if let Some((region, secrets_manager)) = getter_region_and_secrets_manager() {
        tracing::info!("process in destruct to aws lambda url");

        let region_provider = RegionProviderChain::default_provider().or_else(region);

        let shared_config = do_with_bench("load shared config from env", async {
            aws_config::from_env().region(region_provider).load().await
        })
        .await;
        let client = do_with_bench("construct client", async { Client::new(&shared_config) }).await;

        let get_secret_value = client.get_secret_value();
        let secret_id = get_secret_value.secret_id(secrets_manager);

        let sent = do_with_bench("send", async { secret_id.send().await }).await;
        let resp = sent.unwrap_or(GetSecretValueOutput::builder().build());

        let value: &str = resp
            .secret_string()
            .ok_or_else(|| Error::FailureGetSecretString)?;
        let secret_info: Value = serde_json::from_str(value)?;

        tracing::info!("process in finish to aws lambda url");
        return make_url_from_value(secret_info);
    }

    return Err(Error::FailureGetSecretString.into());
}
async fn establish_connection<'a, F>(f: F) -> Option<sqlx::MySqlPool>
where
    F: Fn() -> Option<(&'static str, &'a str)>,
{
    let url = construct_url_for_aws(f)
        .await
        .or_else(|_| make_url_from_env())
        .or_else(|_| make_url_default_local())
        .or_else(|e| {
            tracing::error!("Error to establish connection: {:?}", e);
            Err(e)
        })
        .ok()?;

    let establisher = async {
        sqlx::mysql::MySqlPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .unwrap()
    };

    let pool = do_with_bench("establish connection", establisher).await;
    Some(pool)
}
pub async fn get_connection_cache_or_establish<'a, F>(f: F) -> Option<&'static sqlx::MySqlPool>
where
    F: Fn() -> Option<(&'static str, &'a str)>,
{
    static mut POOL: Option<sqlx::MySqlPool> = None;
    unsafe {
        if POOL.is_none() {
            POOL = establish_connection(f).await;
        }
    }
    unsafe { POOL.as_ref() }
}

#[tokio::test]
async fn test() {
    let _ = get_connection_cache_or_establish(|| Some(("ap-northeast-3", "SecretsManager-02")));
}
