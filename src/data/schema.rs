// @generated automatically by Diesel CLI.

diesel::table! {
    invitations (id) {
        id -> Uuid,
        email -> Varchar,
        expires_at -> Timestamp,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        referral_tokens -> Nullable<Text>,
    }
}

diesel::table! {
    messages (id) {
        id -> Uuid,
        topic_id -> Uuid,
        sort_order -> Int4,
        content -> Text,
        role -> Varchar,
        deleted -> Bool,
        prompt_tokens -> Nullable<Int4>,
        completion_tokens -> Nullable<Int4>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    password_resets (id) {
        id -> Uuid,
        email -> Varchar,
        expires_at -> Timestamp,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    stripe_customers (id) {
        id -> Uuid,
        stripe_id -> Text,
        email -> Varchar,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    topics (id) {
        id -> Uuid,
        user_id -> Uuid,
        resolution -> Text,
        side -> Bool,
        deleted -> Bool,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    users (id) {
        id -> Uuid,
        email -> Varchar,
        hash -> Varchar,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::joinable!(messages -> topics (topic_id));
diesel::joinable!(topics -> users (user_id));

diesel::allow_tables_to_appear_in_same_query!(
    invitations,
    messages,
    password_resets,
    stripe_customers,
    topics,
    users,
);
