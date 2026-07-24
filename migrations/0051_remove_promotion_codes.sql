DROP TABLE promo_usages;
DROP TABLE promo_codes;

DELETE FROM app_settings WHERE key = 'promo_code_enabled';
