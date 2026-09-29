-- Collapse the overlapping permission CHECK constraints into one registry.
-- 202609081100 re-added chk_operator_permissions_release without dropping
-- chk_operator_permissions_registry, so MariaDB enforced both lists at once and
-- only their shared release.* codes stayed insertable. Bootstrapping an
-- Operations administrator then failed with ERROR 4025 on
-- release.environment.write, which only the release registry allows.
-- A single restartable ALTER drops every known constraint name before adding the
-- merged registry, so the constraint survives failure and retry.
ALTER TABLE operator_permissions
    DROP CONSTRAINT IF EXISTS chk_operator_permissions_release,
    DROP CONSTRAINT IF EXISTS chk_operator_permissions_registry,
    ADD CONSTRAINT chk_operator_permissions_registry CHECK (permission_code IN (
        'release.read', 'release.build', 'release.download',
        'release.publish.request', 'release.publish.approve', 'release.publish.execute',
        'release.environment.write', 'release.environment.upgrade',
        'commercial.plan.read', 'commercial.plan.write',
        'commercial.order.read', 'commercial.order.write', 'commercial.payment.confirm',
        'commercial.fulfillment.read', 'commercial.fulfillment.approve',
        'commercial.distribution.read', 'commercial.distribution.approve', 'commercial.license.issue',
        'commercial.catalog.read', 'commercial.catalog.approve', 'commercial.catalog.export',
        'commercial.publication.read', 'commercial.publication.prepare', 'commercial.publication.accept'
    ));
