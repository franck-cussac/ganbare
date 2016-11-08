CREATE TABLE answer_data (
	id SERIAL PRIMARY KEY,
	user_id SERIAL REFERENCES users,
	correct_qa_id SERIAL REFERENCES question_answers,
	answered_qa_id SERIAL REFERENCES question_answers,
	answered_date TIMESTAMPTZ NOT NULL DEFAULT current_timestamp,
	answer_time_ms INTEGER,
	correct BOOLEAN
);
