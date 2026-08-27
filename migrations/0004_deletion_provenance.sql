-- Migration 0004: Explicit Deletion and Message Provenance
-- Adds explicit provenance version and durable deletion & historical message sweep metrics to sync_integrity_reports

ALTER TABLE sync_integrity_reports ADD COLUMN provenance_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE sync_integrity_reports ADD COLUMN deletion_reconciliation_performed INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sync_integrity_reports ADD COLUMN deletion_reconciliation_complete INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sync_integrity_reports ADD COLUMN deletion_event_gap_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sync_integrity_reports ADD COLUMN deletion_tombstones_reconciled INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sync_integrity_reports ADD COLUMN historical_message_reconciliation_performed INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sync_integrity_reports ADD COLUMN historical_message_reconciliation_complete INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sync_integrity_reports ADD COLUMN historical_message_gap_count INTEGER NOT NULL DEFAULT 0;
