ALTER TABLE active_turns ADD COLUMN IF NOT EXISTS continuation boolean NOT NULL DEFAULT false;
