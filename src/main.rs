#[macro_use]
extern crate diesel;

use actix_cors::Cors;
use actix_identity::IdentityMiddleware;
use actix_session::{config::PersistentSession, storage::RedisSessionStore, SessionMiddleware};
use actix_web::{cookie::Key, middleware, web, App, HttpServer};
use diesel::{prelude::*, r2d2};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use std;
use time;

mod data;
mod errors;
mod handlers;
mod services;

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations");
pub const SECONDS_IN_MINUTE: u64 = 60;
pub const SECONDS_IN_HOUR: u64 = 60 * SECONDS_IN_MINUTE;
pub const SECONDS_IN_DAY: u64 = 24 * SECONDS_IN_HOUR;

fn run_migrations(conn: &mut impl MigrationHarness<diesel::pg::Pg>) {
    conn.run_pending_migrations(MIGRATIONS).unwrap();
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv::dotenv().ok();
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    let redis_store = RedisSessionStore::new("redis://localhost:6379")
        .await
        .unwrap();

    // create db connection pool
    let manager = r2d2::ConnectionManager::<PgConnection>::new(database_url);
    let pool: data::models::Pool = r2d2::Pool::builder()
        .build(manager)
        .expect("Failed to create pool.");

    run_migrations(&mut pool.get().unwrap());

    let domain: String = std::env::var("DOMAIN").unwrap_or_else(|_| "localhost".to_string());

    log::info!("starting HTTP server at http://localhost:8080");

    HttpServer::new(move || {
        let cors = Cors::default()
            .allowed_origin("http://localhost:3000")
            .allowed_origin("http://localhost:3001")
            .allowed_origin("https://editor.arguflow.gg")
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
                    .service(
                        web::resource("/totp")
                            .route(web::post().to(handlers::totp_handler::post_totp))
                            .route(web::get().to(handlers::totp_handler::get_totp)),
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
                    ),
            )
    })
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}
