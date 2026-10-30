CREATE TABLE operator_alert_deliveries (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL CHECK(kind IN ('MANUAL_PAYOUT','LIQUIDITY')),
    subject TEXT NOT NULL,
    reason TEXT NOT NULL,
    active INTEGER NOT NULL CHECK(active IN (0,1)),
    created_at INTEGER NOT NULL,
    due_at INTEGER NOT NULL,
    lease_until INTEGER,
    lease_actor TEXT,
    generation INTEGER NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    delivered_at INTEGER,
    delivery_evidence TEXT
) STRICT;
CREATE INDEX operator_alerts_due ON operator_alert_deliveries(delivered_at,due_at,lease_until);
INSERT INTO operator_alert_deliveries(kind,subject,reason,active,created_at,due_at)
SELECT 'MANUAL_PAYOUT',payout_id,reason,1,created_at,created_at FROM payout_alerts;
INSERT INTO operator_alert_deliveries(kind,subject,reason,active,created_at,due_at)
SELECT 'LIQUIDITY',role,reason,active,updated_at,updated_at FROM liquidity_alerts;
CREATE TRIGGER payout_alert_delivery AFTER INSERT ON payout_alerts BEGIN
    INSERT INTO operator_alert_deliveries(kind,subject,reason,active,created_at,due_at)
    VALUES('MANUAL_PAYOUT',NEW.payout_id,NEW.reason,1,NEW.created_at,NEW.created_at);
END;
CREATE TRIGGER liquidity_alert_delivery AFTER INSERT ON liquidity_alerts BEGIN
    INSERT INTO operator_alert_deliveries(kind,subject,reason,active,created_at,due_at)
    VALUES('LIQUIDITY',NEW.role,NEW.reason,NEW.active,NEW.updated_at,NEW.updated_at);
END;
CREATE TRIGGER liquidity_alert_change_delivery AFTER UPDATE ON liquidity_alerts
WHEN OLD.active<>NEW.active OR OLD.reason<>NEW.reason BEGIN
    INSERT INTO operator_alert_deliveries(kind,subject,reason,active,created_at,due_at)
    VALUES('LIQUIDITY',NEW.role,NEW.reason,NEW.active,NEW.updated_at,NEW.updated_at);
END;
