DROP TABLE invitation_uses;
DROP TABLE invitation_codes;

DELETE FROM app_settings WHERE key = 'invitation_required';
