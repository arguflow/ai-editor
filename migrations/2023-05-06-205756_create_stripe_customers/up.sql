-- Your SQL goes here
CREATE TABLE stripe_customers (
  id UUID NOT NULL UNIQUE PRIMARY KEY,
  stripe_id TEXT NOT NULL UNIQUE,
  email VARCHAR(100) NOT NULL UNIQUE,
  created_at TIMESTAMP NOT NULL,
  updated_at TIMESTAMP NOT NULL,
  FOREIGN KEY (email) REFERENCES users(email)
);
