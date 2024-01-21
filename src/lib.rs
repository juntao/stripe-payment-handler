use dotenv::dotenv;
use flowsnet_platform_sdk::logger;
use serde::{ Deserialize, Serialize };
use std::collections::HashMap;
use serde_json::Value;
use webhook_flows::{ create_endpoint, request_handler, send_response };
use std::env;
// use mysql_async::{prelude::*, Opts, OptsBuilder, Conn, Pool, PoolConstraints, PoolOpts, SslOpts};
use mysql_async::{prelude::*, Opts, OptsBuilder, Conn, Pool, PoolConstraints, PoolOpts};

#[derive(Default, Serialize, Deserialize)]
struct OwnerRepo {
    or_id: u64,
    owner_repo: String,
    count: u64,
    sub_id: String,
}
impl OwnerRepo {
    fn new(
        or_id: u64,
        owner_repo: String,
        count: u64,
        sub_id: String,
    ) -> Self {
        Self {
            or_id,
            owner_repo,
            count,
            sub_id,
        }
    }
}

#[no_mangle]
#[tokio::main(flavor = "current_thread")]
pub async fn on_deploy() {
    create_endpoint().await;
}

#[request_handler]
async fn handler(
    _headers: Vec<(String, String)>,
    _subpath: String,
    _qry: HashMap<String, Value>,
    body: Vec<u8>
) {
    dotenv().ok();
    logger::init();
    let json: Value = serde_json::from_slice(&body).unwrap();
    log::info!("Input JSON: {}", serde_json::to_string_pretty(&json).unwrap());
    let event_type = json.get("type").expect("Must have event type").as_str().unwrap();

    if event_type == "checkout.session.completed" {
        let checkout_session = json.get("data").unwrap().get("object").unwrap();
        let sub_id = checkout_session.get("subscription").unwrap().as_str().unwrap();
        let custom_field = checkout_session.get("custom_fields").unwrap().get(0).unwrap();
        let owner_repo = custom_field.get("text").unwrap().get("value").unwrap().as_str().unwrap();

        if sub_id.is_empty() || owner_repo.is_empty() {
            log::warn!("sub_id OR owner_repo is empty");

        } else {
            let pool = get_conn_pool();
            let mut conn = pool.get_conn().await.unwrap();

            let repos = "SELECT or_id, count, sub_id FROM repos WHERE owner_repo=:owner_repo"
              .with(params! {
                "owner_repo" => owner_repo.to_uppercase(),
              }).map(&mut conn, |(or_id, count, sub_id)|
                  OwnerRepo::new(or_id, owner_repo.to_string(), count, sub_id)
              ).await.unwrap();

            if repos.len() < 1 {
                r"INSERT INTO repos (owner_repo, count, sub_id)
                VALUES (:owner_repo, :count, :sub_id)"
                  .with(params! {
                    "owner_repo" => owner_repo.clone().to_uppercase(),
                    "count" => 0,
                    "sub_id" => sub_id,
                  }).ignore(&mut conn).await.unwrap();

            } else {
                r"UPDATE repos SET sub_id=:sub_id WHERE or_id=:or_id"
                  .with(params! {
                    "sub_id" => sub_id,
                    "or_id" => repos[0].or_id,
                  }).ignore(&mut conn).await.unwrap();
            }

            r"INSERT INTO subscription_events (event_type, owner_repo, sub_id, message)
            VALUES (:owner_repo, :count, :sub_id, :message)"
              .with(params! {
                "event_type" => event_type,
                "owner_repo" => owner_repo.clone().to_uppercase(),
                "sub_id" => sub_id,
                "message" => serde_json::to_string_pretty(&json).unwrap(),
              }).ignore(&mut conn).await.unwrap();

            drop(conn);
            pool.disconnect().await.unwrap();
        }

    } else if event_type.starts_with("customer.subscription") {

        let subscription = json.get("data").unwrap().get("object").unwrap();
        let sub_id = subscription.get("id").unwrap().as_str().unwrap();
        let pool = get_conn_pool();
        let mut conn = pool.get_conn().await.unwrap();

        if event_type == "customer.subscription.deleted" {
            r"UPDATE repos SET sub_id='' WHERE sub_id=:sub_id"
              .with(params! {
                "sub_id" => sub_id,
              }).ignore(&mut conn).await.unwrap();
        }

        r"INSERT INTO subscription_events (event_type, owner_repo, sub_id, message)
        VALUES (:owner_repo, :count, :sub_id, :message)"
          .with(params! {
            "event_type" => event_type,
            "owner_repo" => "".to_string(),
            "sub_id" => sub_id,
            "message" => serde_json::to_string_pretty(&json).unwrap(),
          }).ignore(&mut conn).await.unwrap();

        drop(conn);
        pool.disconnect().await.unwrap();
    }

    send_response(200, vec![(String::from("content-type"), String::from("text/plain"))], "".as_bytes().to_vec());
    return;
}

fn get_conn_pool () -> Pool {
    let database_url = std::env::var("DATABASE_URL").unwrap();
    let opts = Opts::from_url(&database_url).unwrap();
    let mut builder = OptsBuilder::from_opts(opts);
    // builder = builder.ssl_opts(SslOpts::default());
    let constraints = PoolConstraints::new(1, 2).unwrap();
    let pool_opts = PoolOpts::default().with_constraints(constraints);
    let pool = Pool::new(builder.pool_opts(pool_opts));
    return pool;
}
