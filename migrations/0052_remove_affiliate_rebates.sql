DROP TABLE affiliate_rebates;
DROP TABLE affiliate_transfers;
DROP TABLE affiliate_invites;
DROP TABLE affiliate_profiles;

DELETE FROM app_settings
WHERE key IN (
    'affiliate_enabled',
    'affiliate_rebate_rate_bps',
    'affiliate_rebate_freeze_hours',
    'affiliate_rebate_duration_days',
    'affiliate_rebate_per_invitee_cap_cents'
);
