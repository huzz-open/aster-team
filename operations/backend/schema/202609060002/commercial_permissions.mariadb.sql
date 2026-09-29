-- Expand the explicit permission registry. Do not grant rights to existing users.
-- A single restartable ALTER preserves the constraint across failure/retry.
ALTER TABLE operator_permissions
    DROP CONSTRAINT IF EXISTS chk_operator_permissions_release,
    DROP CONSTRAINT IF EXISTS chk_operator_permissions_registry,
    ADD CONSTRAINT chk_operator_permissions_registry CHECK (permission_code IN (
        'release.read', 'release.build', 'release.download',
        'release.publish.request', 'release.publish.approve', 'release.publish.execute',
        'commercial.plan.read', 'commercial.plan.write',
        'commercial.order.read', 'commercial.order.write'
    ));
