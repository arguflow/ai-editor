use std::borrow::Borrow;
use std::str::FromStr;

use actix_web::web;
use stripe::{
    CheckoutSession, CheckoutSessionMode, CreateCheckoutSession, CreateCheckoutSessionLineItems,
    CreateCustomer, CustomerId, EventObject, EventType, Webhook,
};

use crate::data::models::{Pool, UserPlan};
use crate::diesel::prelude::*;
use crate::handlers::invitation_handler::create_invitation;
use crate::operators::password_reset_operator::get_user_query;
use crate::{data::models::StripeCustomer, errors::DefaultError};

pub async fn create_stripe_checkout_session_operation(
    stripe_customer: Option<StripeCustomer>,
    plan_id: String,
) -> Result<String, DefaultError> {
    let stripe_client = get_stripe_client()?;
    let app_url: String =
        std::env::var("APP_URL").unwrap_or_else(|_| "http://localhost:3000".into());
    let success_url = format!("{}/payment/success", app_url);
    let cancel_url = format!("{}/payment/cancel", app_url);

    let mut params = CreateCheckoutSession::new(&success_url);
    params.cancel_url = Some(&cancel_url);
    params.customer =
        stripe_customer.map(|customer| CustomerId::from_str(&customer.stripe_id).unwrap());
    params.mode = Some(CheckoutSessionMode::Subscription);
    params.line_items = Some(vec![CreateCheckoutSessionLineItems {
        price: Some(plan_id),
        quantity: Some(1),
        ..Default::default()
    }]);

    let checkout_session = CheckoutSession::create(&stripe_client, params)
        .await
        .map_err(|_stripe_error| DefaultError {
            message: "Error creating checkout session, try again",
        })?;
    let checkout_session_url = checkout_session.url.ok_or(DefaultError {
        message: "Error creating checkout session, try again",
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

pub fn get_user_plan_query(
    user_email: String,
    pool: &web::Data<Pool>,
) -> Result<UserPlan, DefaultError> {
    use crate::data::schema::user_plans::dsl::{
        stripe_customer_id as stripe_customer_id_column, user_plans,
    };

    // get the user's stripe customer id from the stripe_customers table
    let stripe_customer_id = get_stripe_customer_query(user_email, pool)?.stripe_id;

    let mut conn = pool.get().unwrap();

    let user_plan = user_plans
        .filter(stripe_customer_id_column.eq(stripe_customer_id))
        .first::<UserPlan>(&mut conn)
        .map_err(|_db_error| DefaultError {
            message: "Error finding user plan, try again",
        })?;

    Ok(user_plan)
}

pub fn create_user_plan_query(
    stripe_customer_id: String,
    plan_name: String,
    pool: &web::Data<Pool>,
) -> Result<UserPlan, DefaultError> {
    use crate::data::schema::user_plans::dsl::user_plans;

    let mut conn = pool.get().unwrap();

    let new_user_plan = UserPlan::from_details(stripe_customer_id, plan_name);

    let inserted_user_plan = diesel::insert_into(user_plans)
        .values(&new_user_plan)
        .get_result(&mut conn)
        .map_err(|_db_error| DefaultError {
            message: "Error inserting new user plan, try again",
        })?;

    Ok(inserted_user_plan)
}

pub fn handle_webhook_query(
    stripe_signature: &str,
    payload: web::Bytes,
    pool: &web::Data<Pool>,
) -> Result<(), DefaultError> {
    let webhook_secret =
        std::env::var("WEBHOOK_SIGNING_SECRET").expect("WEBHOOK_SIGNING_SECRET must be set");
    let silver_plan_id =
        std::env::var("STRIPE_SILVER_PLAN_ID").expect("STRIPE_SILVER_PLAN_ID must be set");
    let gold_plan_id =
        std::env::var("STRIPE_GOLD_PLAN_ID").expect("STRIPE_GOLD_PLAN_ID must be set");

    let payload_str = std::str::from_utf8(payload.borrow()).unwrap();

    if let Ok(event) = Webhook::construct_event(payload_str, stripe_signature, &webhook_secret) {
        match event.type_ {
            EventType::CheckoutSessionCompleted => {
                if let EventObject::CheckoutSession(session) = event.data.object {
                    let stripe_customer = match &session.customer {
                        Some(customer) => customer,
                        None => {
                            let err = DefaultError {
                                message: "Stripe customer id is none",
                            };
                            log::error!("{}", err.message);
                            return Err(err);
                        }
                    };
                    log::info!("Session {:?}", &session);
                    if session.line_items.data.len() != 1 {
                        let err = DefaultError {
                            message: "Session line items length is not 1",
                        };
                        log::error!("{}", err.message);
                        return Err(err);
                    }

                    let plan_price = match session.line_items.data[0].price.clone() {
                        Some(price) => price,
                        None => {
                            let err = DefaultError {
                                message: "Plan price is none",
                            };
                            log::error!("{}", err.message);
                            return Err(err);
                        }
                    };

                    let plan_id = plan_price.id.to_string();
                    match plan_id {
                        id if id == gold_plan_id => create_user_plan_query(
                            stripe_customer.id().to_string(),
                            "gold".to_owned(),
                            pool,
                        ),
                        id if id == silver_plan_id => create_user_plan_query(
                            stripe_customer.id().to_string(),
                            "silver".to_owned(),
                            pool,
                        ),
                        _ => {
                            let err = DefaultError {
                                message: "Plan id is not silver or gold",
                            };
                            log::error!("{}", err.message);
                            return Err(err);
                        }
                    }
                    .map_err(|_db_error| {
                        log::error!("Error creating user plan, try again {:?}", _db_error);

                        DefaultError {
                            message: "Error creating user plan, try again",
                        }
                    })?;

                    let email = session.customer_email.unwrap();
                    log::info!("Customer email {:?}", email);
                    let arguflow_user = get_user_query(&email, pool).ok();
                    if arguflow_user.is_none() {
                        create_invitation(email, "".to_owned(), pool.to_owned())?;
                    }
                }
            }
            EventType::CustomerCreated => {
                if let EventObject::Customer(customer) = event.data.object {
                    if let Some(email) = customer.email {
                        // If they are not in our db now, send invite
                        log::info!("Customer email {:?}", email);
                        let arguflow_user = get_user_query(&email, pool).ok();
                        if arguflow_user.is_none() {
                            create_invitation(email, "".to_owned(), pool.to_owned())?;
                        }
                    }
                }
            }
            _ => {
                log::error!("Unknown event encountered in webhook: {:?}", event.type_);
            }
        }
    } else {
        log::error!("Failed to construct webhook event, ensure your webhook secret is correct.");
    }

    Ok(())
}
