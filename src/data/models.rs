#![allow(clippy::extra_unused_lifetimes)]

use diesel::{r2d2::ConnectionManager, PgConnection};
use openai_dive::v1::resources::chat_completion::{ChatMessage, Role};
use serde::{Deserialize, Serialize};

use super::schema::*;

// type alias to use in multiple places
pub type Pool = r2d2::Pool<ConnectionManager<PgConnection>>;

#[derive(Debug, Serialize, Deserialize, Queryable, Insertable)]
#[diesel(table_name = users)]
pub struct User {
    pub id: uuid::Uuid,
    pub email: String,
    pub hash: String,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

impl User {
    pub fn from_details<S: Into<String>, T: Into<String>>(email: S, pwd: T) -> Self {
        User {
            id: uuid::Uuid::new_v4(),
            email: email.into(),
            hash: pwd.into(),
            created_at: chrono::Local::now().naive_local(),
            updated_at: chrono::Local::now().naive_local(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Queryable, Insertable)]
#[diesel(table_name = invitations)]
pub struct Invitation {
    pub id: uuid::Uuid,
    pub email: String,
    pub expires_at: chrono::NaiveDateTime,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
    pub referral_tokens: Option<String>,
}

// any type that implements Into<String> can be used to create Invitation
impl<T> From<T> for Invitation
where
    T: Into<String>,
{
    fn from(email: T) -> Self {
        Invitation {
            id: uuid::Uuid::new_v4(),
            email: email.into(),
            expires_at: chrono::Local::now().naive_local() + chrono::Duration::minutes(5),
            created_at: chrono::Local::now().naive_local(),
            updated_at: chrono::Local::now().naive_local(),
            referral_tokens: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Queryable, Insertable)]
#[diesel(table_name = password_resets)]
pub struct PasswordReset {
    pub id: uuid::Uuid,
    pub email: String,
    pub expires_at: chrono::NaiveDateTime,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

// any type that implements Into<String> can be used to create PasswordReset
impl<T> From<T> for PasswordReset
where
    T: Into<String>,
{
    fn from(email: T) -> Self {
        PasswordReset {
            id: uuid::Uuid::new_v4(),
            email: email.into(),
            expires_at: chrono::Local::now().naive_local() + chrono::Duration::minutes(5),
            created_at: chrono::Local::now().naive_local(),
            updated_at: chrono::Local::now().naive_local(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Queryable, Insertable)]
#[diesel(table_name = topics)]
pub struct Topic {
    pub id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub resolution: String,
    pub side: bool,
    pub deleted: bool,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

impl Topic {
    pub fn from_details<S: Into<String>, T: Into<uuid::Uuid>>(
        resolution: S,
        user_id: T,
        side: bool,
    ) -> Self {
        Topic {
            id: uuid::Uuid::new_v4(),
            user_id: user_id.into(),
            resolution: resolution.into(),
            side,
            deleted: false,
            created_at: chrono::Local::now().naive_local(),
            updated_at: chrono::Local::now().naive_local(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Queryable, Insertable, Clone)]
#[diesel(table_name = messages)]
pub struct Message {
    pub id: uuid::Uuid,
    pub topic_id: uuid::Uuid,
    pub sort_order: i32,
    pub content: String,
    pub role: String,
    pub deleted: bool,
    pub prompt_tokens: Option<i32>,
    pub completion_tokens: Option<i32>,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

impl From<Message> for ChatMessage {
    fn from(message: Message) -> Self {
        let role = match message.role.as_str() {
            "system" => Role::System,
            "user" => Role::User,
            _ => Role::Assistant,
        };

        ChatMessage {
            role,
            content: message.content,
            name: None,
        }
    }
}

impl Message {
    pub fn from_details<S: Into<String>, T: Into<uuid::Uuid>>(
        content: S,
        topic_id: T,
        sort_order: i32,
        role: String,
        prompt_tokens: Option<i32>,
        completion_tokens: Option<i32>,
    ) -> Self {
        Message {
            id: uuid::Uuid::new_v4(),
            topic_id: topic_id.into(),
            sort_order,
            content: content.into(),
            role,
            deleted: false,
            prompt_tokens,
            completion_tokens,
            created_at: chrono::Local::now().naive_local(),
            updated_at: chrono::Local::now().naive_local(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Queryable, Insertable)]
#[diesel(table_name = stripe_customers)]
pub struct StripeCustomer {
    pub id: uuid::Uuid,
    pub stripe_id: String,
    pub email: String,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

impl StripeCustomer {
    pub fn from_details<S: Into<String>, T: Into<String>>(stripe_id: S, email: T) -> Self {
        StripeCustomer {
            id: uuid::Uuid::new_v4(),
            stripe_id: stripe_id.into(),
            email: email.into(),
            created_at: chrono::Local::now().naive_local(),
            updated_at: chrono::Local::now().naive_local(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SlimUser {
    pub id: uuid::Uuid,
    pub email: String,
}

impl From<User> for SlimUser {
    fn from(user: User) -> Self {
        SlimUser {
            id: user.id,
            email: user.email,
        }
    }
}
