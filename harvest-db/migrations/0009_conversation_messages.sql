CREATE TABLE conversation_messages (
    conv_id text    NOT NULL REFERENCES conversations ON DELETE CASCADE,
    seq     integer NOT NULL,
    payload jsonb   NOT NULL,
    PRIMARY KEY (conv_id, seq)
);

INSERT INTO conversation_messages (conv_id, seq, payload)
SELECT c.id, (m.ord - 1)::integer, m.value
FROM conversations c,
     jsonb_array_elements(replace(c.messages, '\u0000', '')::jsonb) WITH ORDINALITY AS m(value, ord)
WHERE c.messages <> '[]';

UPDATE conversations c
SET message_count = COALESCE((SELECT count(*) FROM conversation_messages m WHERE m.conv_id = c.id), 0);

ALTER TABLE conversations DROP COLUMN messages;
