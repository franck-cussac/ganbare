ALTER TABLE events DROP COLUMN required_group;
ALTER TABLE events DROP COLUMN priority;
DELETE FROM event_experiences WHERE event_id=(SELECT id FROM events WHERE name='sorting_ceremony');
DELETE FROM event_experiences WHERE event_id=(SELECT id FROM events WHERE name='pretest');
DELETE FROM event_experiences WHERE event_id=(SELECT id FROM events WHERE name='posttest');
DELETE FROM events WHERE name='sorting_ceremony';
DELETE FROM events WHERE name='pretest';
DELETE FROM events WHERE name='posttest';
