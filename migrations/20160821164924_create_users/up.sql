CREATE TABLE users (
	id SERIAL PRIMARY KEY,
	email VARCHAR NOT NULL UNIQUE,
	joined timestamptz NOT NULL DEFAULT current_timestamp
);

CREATE TABLE passwords (
	id SERIAL REFERENCES users ON DELETE CASCADE PRIMARY KEY,
	password_hash BYTEA NOT NULL,
	salt BYTEA NOT NULL,
	initial_rounds SMALLINT NOT NULL DEFAULT 0,
	extra_rounds SMALLINT NOT NULL DEFAULT 0
);
