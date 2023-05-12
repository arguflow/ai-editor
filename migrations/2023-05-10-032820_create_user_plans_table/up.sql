-- Your SQL goes here
CREATE TABLE user_plans (
  id UUID NOT NULL UNIQUE PRIMARY KEY,
  stripe_customer_id UNIQUE TEXT NOT NULL,
  plan TEXT NOT NULL,
  created_at TIMESTAMP NOT NULL,
  updated_at TIMESTAMP NOT NULL
);

CREATE INDEX idx_user_plans_stripe_customer_id ON user_plans (stripe_customer_id);
