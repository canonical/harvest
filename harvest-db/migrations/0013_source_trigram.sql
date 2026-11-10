CREATE INDEX IF NOT EXISTS symbols_class_source_trgm
    ON symbols USING gin (lower(source) gin_trgm_ops)
    WHERE label = 'Class';
