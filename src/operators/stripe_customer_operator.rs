use std::str::FromStr;

use actix_web::web;
use stripe::{
    CheckoutSession, CheckoutSessionMode, CreateCheckoutSession, CreateCheckoutSessionLineItems,
    CreateCustomer, CustomerId,
};

use crate::data::models::Pool;
use crate::diesel::prelude::*;
use crate::{data::models::StripeCustomer, errors::DefaultError};

pub async fn create_stripe_checkout_session_query(
    stripe_customer: StripeCustomer,
    plan_id: String,
) -> Result<String, DefaultError> {
    let stripe_client = get_stripe_client()?;
    let app_url: String =
        std::env::var("APP_URL").unwrap_or_else(|_| "http://localhost:3000".into());
    let success_url = format!("{}/payment/success", app_url);
    let cancel_url = format!("{}/payment/cancel", app_url);

    let mut params = CreateCheckoutSession::new(&success_url);
    params.cancel_url = Some(&cancel_url);
    params.customer = Some(
        CustomerId::from_str(&stripe_customer.stripe_id).map_err(|_err| DefaultError {
            message: "Error creating checkout session, Customer's stripe_id is invalid, try again",
        })?,
    );
    params.mode = Some(CheckoutSessionMode::Subscription);
    params.line_items = Some(vec![CreateCheckoutSessionLineItems {
        price: Some(plan_id),
        quantity: Some(1),
        ..Default::default()
    }]);

    let checkout_session = CheckoutSession::create(&stripe_client, params)
        .await
        .map_err(|_stripe_error| {
            log::error!("Error creating checkout session: {:?}", _stripe_error);
            DefaultError {
                message: "Error creating checkout session, try again",
            }
        })?;
    let checkout_session_url = checkout_session.url.ok_or(DefaultError {
        message: "Error creating checkout url, try again",
    })?;

    Ok(checkout_session_url)
}

pub fn get_stripe_customer_query(
    email: String,
    pool: &web::Data<Pool>,
) -> Result<StripeCustomer, DefaultError> {
    use crate::data::schema::stripe_customers::dsl::{
        email as stripe_customer_email, stripe_customers,
    };

    let mut conn = pool.get().unwrap();

    let stripe_customer = stripe_customers
        .filter(stripe_customer_email.eq(email))
        .first::<StripeCustomer>(&mut conn)
        .map_err(|_db_error| DefaultError {
            message: "Error finding stripe customer, try again",
        })?;

    Ok(stripe_customer)
}

pub async fn create_stripe_customer_query(
    email: Option<&str>,
    pool: web::Data<Pool>,
) -> Result<StripeCustomer, DefaultError> {
    use crate::data::schema::stripe_customers::dsl::stripe_customers;

    let stripe_client = get_stripe_client()?;
    let new_full_customer = stripe::Customer::create(
        &stripe_client,
        CreateCustomer {
            email,
            ..Default::default()
        },
    )
    .await
    .map_err(|_stripe_error| DefaultError {
        message: "Error creating new stripe customer, try again",
    })?;

    let new_stripe_customer =
        StripeCustomer::from_details(new_full_customer.id.to_string(), new_full_customer.email);

    let mut conn = pool.get().unwrap();

    let inserted_stripe_customer = diesel::insert_into(stripe_customers)
        .values(&new_stripe_customer)
        .get_result(&mut conn)
        .map_err(|_db_error| DefaultError {
            message: "Error inserting new stripe customer, try again",
        })?;

    Ok(inserted_stripe_customer)
}

pub fn get_stripe_client() -> Result<stripe::Client, DefaultError> {
    let stripe_api_secret_key =
        std::env::var("STRIPE_API_SECRET_KEY").expect("STRIPE_API_SECRET_KEY must be set");
    Ok(stripe::Client::new(stripe_api_secret_key))
}
