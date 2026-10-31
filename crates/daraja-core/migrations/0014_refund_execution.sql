CREATE TABLE refund_execution (
    refund_id TEXT PRIMARY KEY REFERENCES refund_requests(id),
    quote_id TEXT NOT NULL UNIQUE REFERENCES payment_quotes(id),
    actor TEXT NOT NULL,
    reason TEXT NOT NULL,
    created_at INTEGER NOT NULL CHECK(created_at>=0)
) STRICT;
CREATE TRIGGER refund_execution_no_update BEFORE UPDATE ON refund_execution
BEGIN SELECT RAISE(ABORT,'immutable refund execution'); END;
CREATE TRIGGER refund_execution_no_delete BEFORE DELETE ON refund_execution
BEGIN SELECT RAISE(ABORT,'immutable refund execution'); END;
CREATE TRIGGER refund_paid AFTER UPDATE OF released_at ON balance_reservations
WHEN NEW.released_at IS NOT NULL AND OLD.released_at IS NULL
BEGIN
    UPDATE unreconciled_deposits SET resolved_at=NEW.released_at,
        resolution_evidence='REFUND_PAID:' || NEW.payout_id
    WHERE receipt IN (
        SELECT r.receipt FROM refund_reservations r JOIN refund_execution e ON e.refund_id=r.refund_id
        JOIN outbound_payouts p ON json_extract(p.dispatch,'$.quote.id')=e.quote_id
        WHERE p.id=NEW.payout_id AND p.state='Paid'
    ) AND resolved_at IS NULL;
END;
