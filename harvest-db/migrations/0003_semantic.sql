
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_available_extensions WHERE name = 'vector') THEN
        RAISE NOTICE 'pgvector is not available; semantic search stays disabled';
        RETURN;
    END IF;

    EXECUTE 'CREATE EXTENSION IF NOT EXISTS vector';

    EXECUTE 'ALTER TABLE symbols ADD COLUMN IF NOT EXISTS embedding vector(768)';
    EXECUTE 'ALTER TABLE symbols ADD COLUMN IF NOT EXISTS embedding_model text';
    EXECUTE 'ALTER TABLE symbols ADD COLUMN IF NOT EXISTS embedding_source_hash text';
EXCEPTION WHEN OTHERS THEN
    RAISE NOTICE 'semantic schema setup skipped: %', SQLERRM;
END $$;
