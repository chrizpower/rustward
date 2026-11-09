ALTER TABLE launches ADD COLUMN launch_failure TEXT
    CHECK(launch_failure IN ('runtime_missing','runtime_invalid','runtime_changed',
        'runtime_unsafe','runtime_command','work_directory','work_storage'));
CREATE TRIGGER launch_failure_evidence AFTER UPDATE OF launch_failure ON launches
WHEN NEW.review_required=1 BEGIN
    UPDATE launches SET evidence_version=evidence_version+1 WHERE run_id=NEW.run_id;
END;
PRAGMA user_version=6;
