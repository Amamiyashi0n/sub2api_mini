DROP TABLE wxpay_payment_oauth_flows;
DROP TABLE payment_refunds;
DROP TABLE payment_events;
DROP TABLE payment_orders;
DROP TABLE payment_provider_instances;

DELETE FROM app_settings
WHERE key IN ('payment_enabled', 'payment_order_expiry_minutes');
