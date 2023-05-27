#[macro_use]
extern crate diesel;

use actix_cors::Cors;
use actix_identity::IdentityMiddleware;
use actix_session::{config::PersistentSession, storage::RedisSessionStore, SessionMiddleware};
use actix_web::{cookie::Key, middleware, web, App, HttpServer};
use diesel::{prelude::*, r2d2};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use qdrant_client::{
    prelude::*,
    qdrant::{VectorParams, VectorsConfig},
};

use crate::operators::card_operator::get_qdrant_connection;

mod data;
mod errors;
mod handlers;
mod operators;

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations");
pub const SECONDS_IN_MINUTE: u64 = 60;
pub const SECONDS_IN_HOUR: u64 = 60 * SECONDS_IN_MINUTE;
pub const SECONDS_IN_DAY: u64 = 24 * SECONDS_IN_HOUR;

fn run_migrations(conn: &mut impl MigrationHarness<diesel::pg::Pg>) {
    conn.run_pending_migrations(MIGRATIONS).unwrap();
}

#[actix_web::main]
pub async fn main() -> std::io::Result<()> {
    dotenv::dotenv().ok();
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let qdrant_url = std::env::var("QDRANT_URL").expect("QDRANT_URL must be set");
    let redis_url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    println!(
        "Connecting to redis at {}, database, {} qdrant, {}",
        redis_url, database_url, qdrant_url
    );

    // create db connection pool
    let manager = r2d2::ConnectionManager::<PgConnection>::new(database_url);
    let pool: data::models::Pool = r2d2::Pool::builder()
        .build(manager)
        .expect("Failed to create pool.");

    let redis_store = RedisSessionStore::new(redis_url.as_str()).await.unwrap();

    let qdrant_client = get_qdrant_connection().await.unwrap();
    let _ = qdrant_client
        .create_collection(&CreateCollection {
            collection_name: "debate_cards".into(),
            vectors_config: Some(VectorsConfig {
                config: Some(qdrant_client::qdrant::vectors_config::Config::Params(
                    VectorParams {
                        size: 1536,
                        distance: Distance::Cosine.into(),
                        hnsw_config: None,
                        quantization_config: None,
                    },
                )),
            }),
            ..Default::default()
        })
        .await
        .map_err(|err| {
            println!("Failed to create collection: {:?}", err);
        });

    run_migrations(&mut pool.get().unwrap());

    let domain: String = std::env::var("DOMAIN").unwrap_or_else(|_| "localhost".to_string());
    let allowed_origin: String =
        std::env::var("ALLOWED_ORIGIN").unwrap_or_else(|_| "http://localhost:3000".to_string());

    log::info!("starting HTTP server at http://localhost:8090");

    HttpServer::new(move || {
        let cors = Cors::default()
            .allowed_origin(&allowed_origin)
            .allowed_origin("https://vault.arguflow.com")
            .allowed_methods(vec!["GET", "POST", "DELETE", "OPTIONS", "PUT"])
            .allow_any_header()
            .supports_credentials()
            .max_age(3600);

        App::new()
            .app_data(web::Data::new(pool.clone()))
            .wrap(
                IdentityMiddleware::builder()
                    .login_deadline(Some(std::time::Duration::from_secs(SECONDS_IN_DAY)))
                    .visit_deadline(Some(std::time::Duration::from_secs(SECONDS_IN_DAY)))
                    .build(),
            )
            .wrap(cors)
            .wrap(
                SessionMiddleware::builder(
                    redis_store.clone(),
                    Key::from(handlers::register_handler::SECRET_KEY.as_bytes()),
                )
                .session_lifecycle(
                    PersistentSession::default().session_ttl(time::Duration::days(1)),
                )
                .cookie_name("ai-editor".to_owned())
                .cookie_secure(false)
                .cookie_domain(Some(domain.clone()))
                .cookie_path("/".to_owned())
                .build(),
            )
            // enable logger
            .wrap(middleware::Logger::default())
            // everything under '/api/' route
            .service(
                web::scope("/api")
                    .service(
                        web::resource("/invitation")
                            .route(web::post().to(handlers::invitation_handler::post_invitation)),
                    )
                    .service(
                        web::resource("/register/{invitation_id}")
                            .route(web::post().to(handlers::register_handler::register_user)),
                    )
                    .service(
                        web::resource("/auth")
                            .route(web::post().to(handlers::auth_handler::login))
                            .route(web::delete().to(handlers::auth_handler::logout))
                            .route(web::get().to(handlers::auth_handler::get_me)),
                    )
                    .service(web::resource("/password/{email}").route(
                        web::get().to(
                            handlers::password_reset_handler::send_password_reset_email_handler,
                        ),
                    ))
                    .service(
                        web::resource("/password").route(
                            web::post()
                                .to(handlers::password_reset_handler::reset_user_password_handler),
                        ),
                    )
                    .service(
                        web::resource("/topic")
                            .route(web::post().to(handlers::topic_handler::create_topic))
                            .route(web::delete().to(handlers::topic_handler::delete_topic))
                            .route(web::put().to(handlers::topic_handler::update_topic))
                            .route(web::get().to(handlers::topic_handler::get_all_topics)),
                    )
                    .service(
                        web::resource("/message")
                            .route(
                                web::post().to(
                                    handlers::message_handler::create_message_completion_handler,
                                ),
                            )
                            .route(web::put().to(handlers::message_handler::edit_message_handler))
                            .route(
                                web::delete()
                                    .to(handlers::message_handler::regenerate_message_handler),
                            ),
                    )
                    .service(
                        web::resource("/messages/{messages_topic_id}").route(
                            web::get().to(handlers::message_handler::get_all_topic_messages),
                        ),
                    )
                    .service(
                        web::resource("/card")
                            .route(web::post().to(handlers::card_handler::create_card))
                            .route(web::put().to(handlers::card_handler::vote_card))
                    )
                    .service(
                        web::resource("/card/{card_id}")
                            .route(web::get().to(handlers::card_handler::get_card_by_id)),
                    )
                    .service(
                        web::resource("/card/search/")
                            .route(web::post().to(handlers::card_handler::search_card)),
                    )
                    .service(
                        web::resource("/card/search/{page}")
                            .route(web::post().to(handlers::card_handler::search_card)),
                    )
                    .service(
                        web::scope("/stripe")
                            .service(
                                web::resource("/plan")
                                    .route(
                                        web::get().to(handlers::stripe_handler::get_subscription),
                                    )
                                    .route(
                                        web::delete()
                                            .to(handlers::stripe_handler::cancel_subscription),
                                    )
                                    .route(web::put().to(handlers::stripe_handler::change_plan)),
                            )
                            .service(
                                web::resource("/webhook").route(
                                    web::post().to(handlers::stripe_handler::stripe_webhook),
                                ),
                            )
                            .service(
                                web::resource("/{plan_id}").route(
                                    web::get().to(
                                        handlers::stripe_handler::create_stripe_checkout_session,
                                    ),
                                ),
                            ),
                    ),
            )
    })
    .bind(("0.0.0.0", 8090))?
    .run()
    .await
}
