//! The declaration used to build each route is also its audit inventory.
//! Classification is not an authorization permit: handlers and service entry
//! points retain their identity, scope, current License and commit-time checks.
use std::path::Path;

use aster_license_core::catalog::{BusinessOperationId, CapabilityId};
use axum::{Extension, Router, extract::DefaultBodyLimit};
use tower_http::services::{ServeDir, ServeFile};

use super::ControlState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryClass {
    PublicBootstrap(&'static str),
    AuthenticatedSupport(&'static str),
    LicensedOperation(&'static [CapabilityId]),
    RetainedOperation(&'static [CapabilityId]),
    // Only disable/delete branches are retained; enabling still requires active rights.
    ResourceStatusOperation(&'static [CapabilityId]),
    RejectUnknownApi,
    StaticApplication,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HttpEntry {
    pub operation: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub class: EntryClass,
    pub websocket: bool,
}

impl HttpEntry {
    const fn new(
        operation: &'static str,
        method: &'static str,
        path: &'static str,
        class: EntryClass,
        websocket: bool,
    ) -> Self {
        assert!(!operation.is_empty() && !method.is_empty() && !path.is_empty());
        match class {
            EntryClass::PublicBootstrap(reason) | EntryClass::AuthenticatedSupport(reason) => {
                assert!(!reason.is_empty());
            }
            EntryClass::LicensedOperation(capabilities)
            | EntryClass::RetainedOperation(capabilities)
            | EntryClass::ResourceStatusOperation(capabilities) => {
                assert!(!capabilities.is_empty())
            }
            _ => {}
        }
        Self {
            operation,
            method,
            path,
            class,
            websocket,
        }
    }
}

macro_rules! registered_http_routes {
    ($( $method:ident $path:literal => $handler:ident, $class:expr, $websocket:literal $(, limit = $limit:expr)? ; )*) => {
        pub const HTTP_ENTRIES: &[HttpEntry] = &[$(
            HttpEntry::new(stringify!($handler), stringify!($method), $path, $class, $websocket),
        )*];

        pub(super) fn api_router(state: ControlState) -> Router {
            let mut router = Router::<ControlState>::new();
            $(
                let entry = HttpEntry::new(stringify!($handler), stringify!($method), $path, $class, $websocket);
                let method_router: axum::routing::MethodRouter<ControlState> =
                    axum::routing::$method(super::$handler).layer(Extension(entry));
                $(let method_router = method_router.layer(DefaultBodyLimit::max($limit));)?
                router = router.route($path, method_router);
            )*
            router.with_state(state)
        }
    };
}

// These declarations belong only to the private executor listener. RuntimeControl
// wraps the entire router in its installation-token guard, including body parsing.
macro_rules! registered_runtime_routes {
    ($( $method:ident $path:literal => $handler:ident, $reason:literal; )*) => {
        pub const RUNTIME_ENTRIES: &[HttpEntry] = &[$(
            HttpEntry::new(concat!("runtime_", stringify!($handler)), stringify!($method), $path,
                EntryClass::AuthenticatedSupport($reason), false),
        )*];

        pub(super) fn runtime_router(state: super::runtime_control::RuntimeControl) -> Router {
            let mut router = Router::<super::runtime_control::RuntimeControl>::new();
            $(
                let entry = HttpEntry::new(concat!("runtime_", stringify!($handler)), stringify!($method),
                    $path, EntryClass::AuthenticatedSupport($reason), false);
                router = router.route($path, axum::routing::$method(super::runtime_control::$handler)
                    .layer(Extension(entry)));
            )*
            router.with_state(state)
        }
    };
}

registered_runtime_routes! {
    get "/v1/status" => status, "Installation-token protected executor status";
    post "/v1/admission" => admission, "Installation-token protected admission revision change";
    post "/v1/retire" => retire, "Installation-token protected drained process retirement";
    post "/v1/runner-probe" => runner_probe, "Installation-token protected signed Runner probe";
    post "/v1/dependencies" => dependencies, "Installation-token protected dependency inspection";
    post "/v1/readiness" => readiness, "Installation-token protected candidate readiness";
    post "/v1/readiness-models" => readiness_models, "Installation-token protected complete configured model inventory";
}

macro_rules! registered_web_routes {
    ($($operation:ident: $prefix:literal;)* => $spa_operation:ident) => {
        pub const WEB_ENTRIES: &[HttpEntry] = &[
            $(HttpEntry::new(stringify!($operation), "*", $prefix, EntryClass::RejectUnknownApi, false),)*
            HttpEntry::new(stringify!($spa_operation), "get/head", "<fallback>", EntryClass::StaticApplication, false),
        ];

        pub fn web_router(state: ControlState, assets: &Path) -> Router {
            let index = assets.join("index.html");
            let mut router = api_router(state.clone());
            $(router = router.route($prefix, axum::routing::any(|| async { axum::http::StatusCode::NOT_FOUND }));)*
            let router = router.fallback_service(ServeDir::new(assets).fallback(ServeFile::new(index)));
            super::track_requests(router, &state)
        }
    };
}

registered_web_routes! {
    unknown_api: "/api/{*path}";
    unknown_gateway: "/v1/{*path}";
    => application_assets
}

use EntryClass::{
    AuthenticatedSupport as Support, LicensedOperation as Licensed, PublicBootstrap as Bootstrap,
    ResourceStatusOperation as ResourceStatus, RetainedOperation as Retained,
};
const MEMBER: &[CapabilityId] = &[CapabilityId::Member];
const RUNNER: &[CapabilityId] = &[CapabilityId::Runner];
const GATEWAY: &[CapabilityId] = &[CapabilityId::Gateway];
const MEMBER_AND_RUNNER: &[CapabilityId] = &[CapabilityId::Member, CapabilityId::Runner];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn method_inventory_is_unique_and_includes_websocket_and_static_boundaries() {
        let mut operations = BTreeSet::new();
        let mut endpoints = BTreeSet::new();
        for entry in HTTP_ENTRIES
            .iter()
            .chain(WEB_ENTRIES)
            .chain(RUNTIME_ENTRIES)
        {
            assert!(operations.insert(entry.operation));
            assert!(endpoints.insert((entry.method, entry.path)));
        }
        let websocket: Vec<_> = HTTP_ENTRIES
            .iter()
            .filter(|entry| entry.websocket)
            .collect();
        assert_eq!(websocket.len(), 1);
        assert_eq!(websocket[0].path, "/api/runner/channel");
        assert_eq!(websocket[0].class, Licensed(RUNNER));
        assert_eq!(
            WEB_ENTRIES
                .iter()
                .filter(|entry| entry.class == EntryClass::RejectUnknownApi)
                .count(),
            2
        );
        assert_eq!(
            WEB_ENTRIES
                .iter()
                .filter(|entry| entry.class == EntryClass::StaticApplication)
                .count(),
            1
        );
        assert_eq!(
            HTTP_ENTRIES
                .iter()
                .find(|entry| entry.operation == "admin_install_license")
                .unwrap()
                .class,
            Support("Authenticated administrator License import and recovery")
        );
    }
}

registered_http_routes! {
    get "/healthz" => health, Bootstrap("Process liveness without product access"), false;
    get "/api/public/license-state" => public_license_state, Bootstrap("License availability for login and recovery"), false;
    get "/api/system/policy" => protected_policy, Licensed(MEMBER), false;
    post "/api/runner/enroll" => enroll_runner, Licensed(RUNNER), false;
    get "/api/runner/install-package/{platform}/{architecture}" => download_runner_install_package, Licensed(RUNNER), false;
    get "/api/runner/install-package/{platform}/{architecture}/checksum" => runner_install_package_checksum, Licensed(RUNNER), false;
    get "/api/runner/channel" => runner_channel, Licensed(RUNNER), true;
    post "/api/admin/auth/login" => admin_login, Bootstrap("Administrator password authentication"), false;
    post "/api/admin/auth/logout" => admin_logout, Bootstrap("Administrator session invalidation"), false;
    post "/api/admin/auth/password" => admin_change_password, Support("Authenticated administrator password recovery"), false;
    get "/api/admin/me" => admin_me, Support("Authenticated administrator identity and effective License view"), false;
    get "/api/admin/license" => admin_license_status, Support("Authenticated administrator License recovery status"), false;
    post "/api/admin/license/preview" => admin_preview_license, Support("Authenticated administrator License validation preview"), false;
    post "/api/admin/license" => admin_install_license, Support("Authenticated administrator License import and recovery"), false;
    get "/api/admin/maintenance" => admin_maintenance_status, Support("Authenticated administrator installation diagnostics"), false;
    post "/api/admin/maintenance/upgrade" => admin_queue_upgrade, Support("Authenticated administrator signed-package upgrade and recovery"), false, limit = 1024 * 1024 * 1024;
    delete "/api/admin/maintenance/versions/{version}" => admin_queue_version_delete, Support("Authenticated administrator historical installation cleanup"), false;
    get "/api/admin/overview" => admin_overview, Support("Authenticated aggregate history retained after expiry"), false;
    get "/api/admin/consumption-logs" => admin_consumption_logs, Retained(MEMBER), false;
    get "/api/admin/consumption-logs/members" => admin_consumption_member_options, Retained(MEMBER), false;
    get "/api/admin/audit-events" => admin_audit_events, Support("Authenticated administrator integrity-checked audit history"), false;
    get "/api/admin/settings" => admin_settings, Support("Existing authenticated installation settings read; no model execution"), false;
    put "/api/admin/settings" => update_admin_settings, Support("Authenticated installation configuration with integrity and audit checks"), false;
    get "/api/admin/billing/settings" => admin_billing_settings, Support("Authenticated local billing currency and exchange configuration read"), false;
    put "/api/admin/billing/settings" => update_admin_billing_settings, Support("Authenticated local billing configuration with integrity and audit checks"), false;
    get "/api/admin/billing/prices" => admin_billing_prices, Support("Authenticated public-model price history read"), false;
    put "/api/admin/billing/prices" => update_admin_billing_prices, Support("Authenticated public-model price version append with integrity and audit checks"), false;
    post "/api/admin/billing/prices/sync" => sync_admin_billing_prices, Support("Authenticated verified official public-model price sync with dated built-in fallback"), false;
    get "/api/admin/billing/overview" => admin_billing_overview, Retained(MEMBER), false;
    get "/api/admin/users/{identity_id}/money" => admin_member_money, Retained(MEMBER), false;
    post "/api/admin/users/{identity_id}/money/grants" => grant_admin_member_money, Licensed(MEMBER), false;
    get "/api/admin/users/{identity_id}/billing-policy" => admin_member_billing_policy, Retained(MEMBER), false;
    put "/api/admin/users/{identity_id}/billing-policy" => update_admin_member_billing_policy, Licensed(MEMBER), false;
    get "/api/admin/quota-requests" => admin_quota_requests, Retained(MEMBER), false;
    patch "/api/admin/quota-requests/{request_id}" => review_member_quota_request, Licensed(MEMBER), false;
    get "/api/admin/vouchers" => admin_vouchers, Retained(MEMBER), false;
    post "/api/admin/vouchers" => create_vouchers, Licensed(MEMBER), false;
    get "/api/admin/vouchers/recipients" => admin_voucher_recipients, Retained(MEMBER), false;
    delete "/api/admin/vouchers/{voucher_id}" => delete_voucher, Licensed(MEMBER), false;
    get "/api/admin/users" => list_members, Retained(MEMBER), false;
    post "/api/admin/users" => create_member, Licensed(MEMBER), false;
    post "/api/admin/users/batch" => create_members_batch, Licensed(MEMBER), false;
    patch "/api/admin/users/{identity_id}" => update_member, ResourceStatus(MEMBER), false;
    get "/api/admin/users/{identity_id}/model-access" => admin_member_model_access, Retained(MEMBER), false;
    put "/api/admin/users/{identity_id}/model-access" => update_member_model_access, Licensed(MEMBER), false;
    delete "/api/admin/users/{identity_id}" => delete_member, Retained(MEMBER), false;
    post "/api/admin/users/{identity_id}/password-reset" => reset_member_password, Retained(MEMBER), false;
    get "/api/admin/users/{identity_id}/quota-adjustments" => list_member_quota_adjustments, Retained(MEMBER), false;
    post "/api/admin/users/{identity_id}/quota-adjustments" => grant_member_quota, Licensed(MEMBER), false;
    get "/api/admin/users/{identity_id}/image-quotas" => admin_image_quotas, Retained(MEMBER), false;
    post "/api/admin/users/{identity_id}/image-quotas/{model_id}/adjust" => admin_adjust_image_quota, Licensed(MEMBER), false;
    get "/api/admin/runners" => list_runners, Retained(RUNNER), false;
    get "/api/admin/runners/connection" => admin_runner_connection, Retained(RUNNER), false;
    post "/api/admin/runners/enrollments" => create_runner_enrollment, Licensed(RUNNER), false;
    patch "/api/admin/runners/{runner_id}" => update_runner, ResourceStatus(RUNNER), false;
    delete "/api/admin/runners/{runner_id}" => delete_runner, Retained(RUNNER), false;
    post "/api/admin/runners/{runner_id}/test" => test_runner, Licensed(RUNNER), false;
    get "/api/admin/upstream-providers" => list_upstream_providers, Retained(GATEWAY), false;
    get "/api/admin/plugins/status" => admin_plugin_status, Retained(GATEWAY), false;
    post "/api/admin/plugins/preflight" => admin_plugin_preflight, Retained(GATEWAY), false, limit = 1048576;
    post "/api/admin/plugins/candidate" => admin_plugin_candidate, Licensed(GATEWAY), false, limit = 16777216;
    post "/api/admin/plugins/activate-version" => admin_plugin_switch_version, Licensed(GATEWAY), false;
    get "/api/admin/upstream-connections" => admin_upstream_connections, Retained(GATEWAY), false;
    post "/api/admin/upstream-connections" => create_upstream_connection, Licensed(GATEWAY), false;
    patch "/api/admin/upstream-connections/{connection_id}" => update_upstream_connection, Licensed(GATEWAY), false;
    put "/api/admin/upstream-connections/{connection_id}/credential" => rotate_upstream_connection_credential, Licensed(GATEWAY), false;
    post "/api/admin/upstream-connections/{connection_id}/verify" => verify_upstream_connection, Licensed(BusinessOperationId::UpstreamSync.required_capabilities()), false;
    post "/api/admin/upstream-connections/{connection_id}/models/sync" => sync_upstream_connection_models, Licensed(BusinessOperationId::UpstreamSync.required_capabilities()), false;
    post "/api/admin/upstream-connections/{connection_id}/models" => add_upstream_connection_models, Licensed(GATEWAY), false;
    get "/api/admin/upstream-connections/{connection_id}/capabilities" => admin_upstream_connection_capabilities, Retained(GATEWAY), false;
    post "/api/admin/upstream-providers/{provider_id}/enrollments" => start_upstream_enrollment, Licensed(BusinessOperationId::UpstreamAuthorize.required_capabilities()), false;
    post "/api/admin/upstream-enrollments/{enrollment_id}/actions" => advance_upstream_enrollment, Licensed(BusinessOperationId::UpstreamAuthorize.required_capabilities()), false;
    delete "/api/admin/upstream-enrollments/{enrollment_id}" => cancel_upstream_enrollment, Retained(GATEWAY), false;
    get "/api/admin/upstream-accounts" => list_upstream_accounts, Retained(GATEWAY), false;
    patch "/api/admin/upstream-accounts/{account_id}" => update_upstream_account, ResourceStatus(GATEWAY), false;
    delete "/api/admin/upstream-accounts/{account_id}" => delete_upstream_account, Retained(GATEWAY), false;
    get "/api/admin/upstream-accounts/{account_id}/credentials" => list_upstream_credentials, Retained(GATEWAY), false;
    post "/api/admin/upstream-accounts/{account_id}/credentials/{credential_id}/refresh" => refresh_upstream_credential, Licensed(BusinessOperationId::UpstreamRefresh.required_capabilities()), false;
    post "/api/admin/upstream-accounts/{account_id}/models/sync" => sync_upstream_models, Licensed(BusinessOperationId::UpstreamSync.required_capabilities()), false;
    get "/api/admin/models" => admin_models, Retained(GATEWAY), false;
    get "/api/admin/models/{model_id}/capabilities" => admin_model_capabilities, Retained(GATEWAY), false;
    patch "/api/admin/models/{model_id}" => update_admin_model, Licensed(GATEWAY), false;
    post "/api/member/auth/login" => member_login, Bootstrap("Member password authentication with retained signed member capability"), false;
    post "/api/member/auth/logout" => member_logout, Bootstrap("Member session invalidation"), false;
    post "/api/member/auth/password" => member_change_password, Retained(MEMBER), false;
    get "/api/member/me" => member_me, Retained(MEMBER), false;
    get "/api/member/money" => member_money, Retained(MEMBER), false;
    get "/api/member/models" => member_models, Retained(MEMBER), false;
    get "/api/member/models/{model_id}/capabilities" => member_model_capabilities, Retained(MEMBER), false;
    get "/api/member/model-access" => member_model_access, Retained(MEMBER), false;
    get "/api/member/docs" => member_docs, Retained(MEMBER), false;
    get "/api/member/claude-cli/settings" => member_claude_cli_settings, Retained(MEMBER), false;
    get "/api/member/asterctl/artifacts" => member_asterctl_artifacts, Retained(MEMBER), false;
    get "/api/member/asterctl/artifacts/{artifact_id}/download" => download_member_asterctl_artifact, Retained(MEMBER), false;
    get "/api/member/usage-summary" => member_usage_summary, Retained(MEMBER), false;
    get "/api/member/usage-logs" => member_usage_logs, Retained(MEMBER), false;
    get "/api/member/ledger" => member_ledger, Retained(MEMBER), false;
    get "/api/member/quota-requests" => member_quota_requests, Retained(MEMBER), false;
    get "/api/member/image-quotas" => member_image_quotas, Retained(MEMBER), false;
    get "/api/member/image-ledger" => member_image_ledger, Retained(MEMBER), false;
    post "/api/member/quota-requests" => create_member_quota_request, Licensed(MEMBER), false;
    patch "/api/member/quota-requests/{request_id}" => update_member_quota_request, Licensed(MEMBER), false;
    delete "/api/member/quota-requests/{request_id}" => withdraw_member_quota_request, Licensed(MEMBER), false;
    get "/api/member/vouchers" => member_vouchers, Retained(MEMBER), false;
    post "/api/member/vouchers/redeem" => redeem_member_voucher, Licensed(MEMBER), false;
    get "/api/member/keys" => list_api_keys, Retained(MEMBER), false;
    post "/api/member/keys" => create_api_key, Licensed(MEMBER), false;
    post "/api/member/keys/{id}/revoke" => revoke_api_key, Retained(MEMBER), false;
    get "/v1/models" => gateway_models, Licensed(MEMBER), false;
    get "/v1/models/{id}" => gateway_model, Licensed(MEMBER), false;
    get "/v1/claude-cli/settings" => gateway_claude_cli_settings, Licensed(MEMBER), false;
    post "/v1/responses" => gateway_responses, Licensed(MEMBER_AND_RUNNER), false, limit = 16 * 1024 * 1024;
    post "/v1/chat/completions" => gateway_chat_completions, Licensed(MEMBER_AND_RUNNER), false, limit = 16 * 1024 * 1024;
    post "/v1/messages" => gateway_anthropic_messages, Licensed(MEMBER_AND_RUNNER), false, limit = 16 * 1024 * 1024;
    post "/v1/images/generations" => gateway_image_generations, Licensed(MEMBER_AND_RUNNER), false;
    post "/v1/images/edits" => gateway_image_edits, Licensed(MEMBER_AND_RUNNER), false, limit = super::gateway::images::MAX_IMAGE_EDIT_BODY_BYTES;
}
