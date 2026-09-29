package httpapi

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"log/slog"
	"mime"
	"net/http"
	"strconv"
	"strings"
	"time"

	"aster.local/team/operations/backend/internal/apierrors"
	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/domain"
	"aster.local/team/operations/backend/internal/ports"
)

const (
	sessionCookieName = "aster_operations_session"
	csrfCookieName    = "aster_operations_csrf"
)

type Config struct {
	CookieSecure   bool
	TrustedOrigins map[string]struct{}
	BodyMaxSize    int64
}

type API struct {
	service *application.Service
	health  func(context.Context) error
	logger  *slog.Logger
	config  Config
}

func New(service *application.Service, health func(context.Context) error, logger *slog.Logger, cfg Config) http.Handler {
	api := &API{service: service, health: health, logger: logger, config: cfg}
	mux := http.NewServeMux()
	mux.HandleFunc("GET /api/operations/v1/upgrade-environments", api.listUpgradeEnvironments)
	mux.HandleFunc("POST /api/operations/v1/upgrade-environments", api.createUpgradeEnvironment)
	mux.HandleFunc("PUT /api/operations/v1/upgrade-environments/{environmentID}/credentials", api.rotateUpgradeCredentials)
	mux.HandleFunc("POST /api/operations/v1/upgrade-environments/{environmentID}/inspect", api.inspectUpgradeEnvironment)
	mux.HandleFunc("GET /api/operations/v1/environment-upgrades", api.listEnvironmentUpgrades)
	mux.HandleFunc("POST /api/operations/v1/environment-upgrades", api.createEnvironmentUpgrade)
	mux.HandleFunc("GET /api/operations/v1/environment-upgrades/{upgradeID}", api.getEnvironmentUpgrade)
	mux.HandleFunc("GET /health", api.healthHandler)
	mux.HandleFunc("GET /api/operations/v1/health", api.healthHandler)
	mux.HandleFunc("POST /api/operations/v1/session", api.loginHandler)
	mux.HandleFunc("GET /api/operations/v1/session", api.sessionHandler)
	mux.HandleFunc("DELETE /api/operations/v1/session", api.logoutHandler)
	mux.HandleFunc("PUT /api/operations/v1/session/password", api.changePasswordHandler)
	mux.HandleFunc("GET /api/operations/v1/overview", api.overviewHandler)
	mux.HandleFunc("GET /api/operations/v1/customers", api.listCustomersHandler)
	mux.HandleFunc("POST /api/operations/v1/customers", api.createCustomerHandler)
	mux.HandleFunc("GET /api/operations/v1/customers/{customerID}", api.getCustomerProfileHandler)
	mux.HandleFunc("PATCH /api/operations/v1/customers/{customerID}", api.updateCustomerHandler)
	mux.HandleFunc("POST /api/operations/v1/customers/{customerID}/contacts", api.createContactHandler)
	mux.HandleFunc("PUT /api/operations/v1/customers/{customerID}/billing-profile", api.upsertBillingProfileHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/plans", api.commercialPlansHandler)
	mux.HandleFunc("POST /api/operations/v1/commercial/catalogs/preview", api.previewPublicCatalogHandler)
	mux.HandleFunc("POST /api/operations/v1/commercial/publications", api.preparePublicationHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/publications", api.listPublicationsHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/publications/{publicationID}", api.getPublicationHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/publications/{publicationID}/failures", api.listPublicationFailuresHandler)
	mux.HandleFunc("POST /api/operations/v1/commercial/publications/{publicationID}/accept", api.acceptPublicationHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/publication-heads/{environment}", api.getPublicationHeadHandler)
	mux.HandleFunc("POST /api/operations/v1/commercial/catalogs", api.approvePublicCatalogHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/catalogs", api.listCatalogApprovalsHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/catalogs/{catalogID}", api.getCatalogApprovalHandler)
	mux.HandleFunc("POST /api/operations/v1/commercial/catalogs/{catalogID}/export", api.exportPublicCatalogHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/catalogs/{catalogID}/download", api.downloadPublicCatalogHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/plan-drafts", api.listPlanDraftsHandler)
	mux.HandleFunc("POST /api/operations/v1/commercial/plan-drafts", api.savePlanDraftHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/plan-drafts/{draftID}", api.getPlanDraftHandler)
	mux.HandleFunc("POST /api/operations/v1/commercial/plan-drafts/freeze", api.freezePlanDraftHandler)
	mux.HandleFunc("POST /api/operations/v1/commercial/plans/versions", api.freezeCommercialPlanHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/plans/{planID}/versions/{version}", api.getCommercialPlanHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/plans/{planID}", api.getCurrentCommercialPlanHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/orders", api.commercialOrdersHandler)
	mux.HandleFunc("POST /api/operations/v1/commercial/orders", api.createCommercialOrderHandler)
	mux.HandleFunc("POST /api/operations/v1/commercial/quotation-orders", api.createQuotationOrderHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/quotation-sources/{reference}", api.getQuotationSourceHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/orders/{orderID}", api.getCommercialOrderHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/orders/{orderID}/payment-context", api.commercialPaymentContextHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/orders/{orderID}/fulfillment-context", api.paidFulfillmentContextHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/orders/{orderID}/fulfillment", api.paidFulfillmentForOrderHandler)
	mux.HandleFunc("POST /api/operations/v1/commercial/orders/{orderID}/fulfillment", api.approvePaidFulfillmentHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/paid-lifecycle-sources/{kind}/{sourceID}", api.getPaidLifecycleSourceHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/paid-fulfillments/{fulfillmentID}", api.getPaidFulfillmentHandler)
	mux.HandleFunc("POST /api/operations/v1/commercial/paid-fulfillments/{fulfillmentID}/issue", api.issuePaidFulfillmentHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/paid-fulfillments/{fulfillmentID}/license", api.downloadPaidFulfillmentHandler)
	mux.HandleFunc("POST /api/operations/v1/commercial/paid-fulfillments/{fulfillmentID}/redeliveries", api.recordPaidRedeliveryHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/paid-redeliveries/{redeliveryID}", api.getPaidRedeliveryHandler)
	mux.HandleFunc("POST /api/operations/v1/commercial/paid-fulfillments/{fulfillmentID}/transfers", api.approvePaidTransferHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/paid-fulfillments/{fulfillmentID}/transfers/latest", api.getLatestPaidTransferHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/paid-transfers/{transferID}", api.getPaidTransferHandler)
	mux.HandleFunc("POST /api/operations/v1/commercial/paid-transfers/{transferID}/issue", api.issuePaidTransferHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/paid-transfers/{transferID}/license", api.downloadPaidTransferHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/orders/{orderID}/payment", api.getCommercialPaymentHandler)
	mux.HandleFunc("POST /api/operations/v1/commercial/orders/{orderID}/payment", api.confirmCommercialPaymentHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/distributions", api.freeDistributionsHandler)
	mux.HandleFunc("POST /api/operations/v1/commercial/distributions", api.approveFreeDistributionHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/distributions/{distributionID}", api.getFreeDistributionHandler)
	mux.HandleFunc("POST /api/operations/v1/commercial/distributions/{distributionID}/issue", api.issueFreeDistributionHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/distributions/{distributionID}/download", api.downloadFreeDistributionHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/issuers", api.v2IssuerProfilesHandler)
	mux.HandleFunc("GET /api/operations/v1/commercial/environment", api.commercialEnvironmentHandler)

	mux.HandleFunc("GET /api/operations/v1/backup-history", api.listBackupHistoryHandler)
	mux.HandleFunc("POST /api/operations/v1/exports", api.exportOperationsHandler)
	mux.HandleFunc("GET /api/operations/v1/audit-events", api.listAuditEventsHandler)
	mux.HandleFunc("GET /api/operations/v1/release-artifacts", api.listReleaseArtifactsHandler)
	mux.HandleFunc("POST /api/operations/v1/release-artifacts", api.importReleaseArtifactHandler)
	mux.HandleFunc("GET /api/operations/v1/release-artifacts/{artifactID}/download", api.downloadReleaseArtifactHandler)
	mux.HandleFunc("GET /api/operations/v1/release-tasks", api.listReleaseTasksHandler)
	mux.HandleFunc("POST /api/operations/v1/release-tasks", api.createReleaseTaskHandler)
	mux.HandleFunc("GET /api/operations/v1/release-tasks/{taskID}", api.getReleaseTaskHandler)
	mux.HandleFunc("POST /api/operations/v1/release-tasks/{taskID}/sync", api.syncReleaseTaskHandler)
	mux.HandleFunc("POST /api/operations/v1/release-tasks/{taskID}/retry", api.retryReleaseTaskHandler)
	mux.HandleFunc("POST /api/operations/v1/release-tasks/{taskID}/artifacts/{artifactID}/reverify", api.reverifyReleaseTaskHandler)
	mux.HandleFunc("GET /api/operations/v1/release-capabilities", api.releaseCapabilitiesHandler)
	mux.HandleFunc("GET /api/operations/v1/release-publish-requests", api.listReleasePublishRequestsHandler)
	mux.HandleFunc("POST /api/operations/v1/release-artifacts/{artifactID}/publish-requests", api.requestReleasePublishHandler)
	mux.HandleFunc("POST /api/operations/v1/release-publish-requests/{requestID}/decision", api.decideReleasePublishHandler)
	mux.HandleFunc("POST /api/operations/v1/release-publish-requests/{requestID}/execute", api.executeReleasePublishHandler)

	return api.middleware(mux)
}

func (api *API) middleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		requestID := request.Header.Get("X-Request-ID")
		if requestID == "" {
			requestID = randomRequestID()
		}
		response.Header().Set("X-Request-ID", requestID)
		response.Header().Set("X-Content-Type-Options", "nosniff")
		response.Header().Set("X-Frame-Options", "DENY")
		response.Header().Set("Referrer-Policy", "same-origin")
		defer func() {
			if recovered := recover(); recovered != nil {
				number := writeError(response, http.StatusInternalServerError, "INTERNAL_ERROR", "服务内部错误")
				api.logger.Error("operations request panicked", "request_id", requestID, "method", request.Method,
					"path", request.URL.Path, "code", "INTERNAL_ERROR", "number", number, "panic", recovered)
				return
			}
			if numberText := response.Header().Get(apierrors.NumberHeader); numberText != "" {
				number, _ := strconv.ParseInt(numberText, 10, 64)
				api.logger.Warn("operations request rejected", "request_id", requestID, "method", request.Method,
					"path", request.URL.Path, "number", number)
			}
		}()
		next.ServeHTTP(response, request)
	})
}

func (api *API) healthHandler(response http.ResponseWriter, request *http.Request) {
	if err := api.health(request.Context()); err != nil {
		writeError(response, http.StatusServiceUnavailable, "DATABASE_UNAVAILABLE", "运营数据库暂时不可用")
		return
	}
	writeJSON(response, http.StatusOK, map[string]string{"status": "ok"})
}

func (api *API) loginHandler(response http.ResponseWriter, request *http.Request) {
	if !api.validOrigin(request) {
		writeError(response, http.StatusForbidden, "ORIGIN_FORBIDDEN", "请求来源不受信任")
		return
	}
	var input struct {
		Email    string `json:"email"`
		Password string `json:"password"`
	}
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	tokens, err := api.service.Login(request.Context(), input.Email, input.Password)
	if err != nil {
		if errors.Is(err, application.ErrInvalidCredentials) {
			writeError(response, http.StatusUnauthorized, "INVALID_CREDENTIALS", "邮箱或密码错误")
			return
		}
		api.logger.Error("operations login failed", "error", err)
		writeError(response, http.StatusInternalServerError, "LOGIN_FAILED", "登录失败")
		return
	}
	api.setSessionCookies(response, tokens)
	writeJSON(response, http.StatusOK, map[string]any{"operator": tokens.Operator, "expires_at": tokens.ExpiresAt})
}

func (api *API) sessionHandler(response http.ResponseWriter, request *http.Request) {
	operator, ok := api.authenticate(response, request)
	if !ok {
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{"operator": operator.Operator, "expires_at": operator.ExpiresAt})
}

func (api *API) logoutHandler(response http.ResponseWriter, request *http.Request) {
	operator, token, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	if err := api.service.Logout(request.Context(), token); err != nil {
		api.logger.Error("operations logout failed", "operator_id", operator.ID, "error", err)
		writeError(response, http.StatusInternalServerError, "LOGOUT_FAILED", "退出失败")
		return
	}
	api.clearSessionCookies(response)
	response.WriteHeader(http.StatusNoContent)
}

func (api *API) changePasswordHandler(response http.ResponseWriter, request *http.Request) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	var input struct {
		CurrentPassword string `json:"current_password"`
		NewPassword     string `json:"new_password"`
	}
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	if err := api.service.ChangePassword(request.Context(), operator, input.CurrentPassword, input.NewPassword); err != nil {
		if errors.Is(err, application.ErrInvalidCredentials) {
			writeError(response, http.StatusUnauthorized, "CURRENT_PASSWORD_INVALID", "当前密码错误")
			return
		}
		if errors.Is(err, application.ErrValidation) {
			writeError(response, http.StatusBadRequest, "PASSWORD_INVALID", err.Error())
			return
		}
		api.logger.Error("change operator password failed", "operator_id", operator.ID, "error", err)
		writeError(response, http.StatusInternalServerError, "PASSWORD_CHANGE_FAILED", "修改密码失败")
		return
	}
	response.WriteHeader(http.StatusNoContent)
}

func (api *API) overviewHandler(response http.ResponseWriter, request *http.Request) {
	if _, ok := api.authenticate(response, request); !ok {
		return
	}
	overview, err := api.service.Overview(request.Context())
	if err != nil {
		api.logger.Error("load operations overview failed", "error", err)
		writeError(response, http.StatusInternalServerError, "OVERVIEW_FAILED", "读取运营概览失败")
		return
	}
	writeJSON(response, http.StatusOK, overview)
}

func (api *API) listCustomersHandler(response http.ResponseWriter, request *http.Request) {
	if _, ok := api.authenticate(response, request); !ok {
		return
	}
	limit := 50
	if raw := request.URL.Query().Get("limit"); raw != "" {
		parsed, err := strconv.Atoi(raw)
		if err != nil || parsed < 1 || parsed > 100 {
			writeError(response, http.StatusBadRequest, "INVALID_LIMIT", "limit 必须在 1 到 100 之间")
			return
		}
		limit = parsed
	}
	customers, next, err := api.service.ListCustomers(request.Context(), request.URL.Query().Get("after"), limit)
	if err != nil {
		api.logger.Error("list operations customers failed", "error", err)
		writeError(response, http.StatusInternalServerError, "CUSTOMERS_LIST_FAILED", "读取客户列表失败")
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{"items": customers, "next": next})
}

func (api *API) createCustomerHandler(response http.ResponseWriter, request *http.Request) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	var input domain.CustomerInput
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	customer, err := api.service.CreateCustomer(request.Context(), input, operator.ID)
	if err != nil {
		if errors.Is(err, application.ErrValidation) {
			writeError(response, http.StatusBadRequest, "VALIDATION_FAILED", err.Error())
			return
		}
		api.logger.Error("create operations customer failed", "operator_id", operator.ID, "error", err)
		writeError(response, http.StatusInternalServerError, "CUSTOMER_CREATE_FAILED", "创建客户失败")
		return
	}
	writeJSON(response, http.StatusCreated, customer)
}

func (api *API) getCustomerProfileHandler(response http.ResponseWriter, request *http.Request) {
	if _, ok := api.authenticate(response, request); !ok {
		return
	}
	profile, err := api.service.GetCustomerProfile(request.Context(), request.PathValue("customerID"))
	if err != nil {
		api.writeBusinessError(response, "CUSTOMER_PROFILE_FAILED", "读取客户档案失败", err)
		return
	}
	writeJSON(response, http.StatusOK, profile)
}

func (api *API) updateCustomerHandler(response http.ResponseWriter, request *http.Request) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	var input domain.CustomerInput
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	customer, err := api.service.UpdateCustomer(request.Context(), request.PathValue("customerID"), input, operator.ID)
	if err != nil {
		api.writeBusinessError(response, "CUSTOMER_UPDATE_FAILED", "更新客户失败", err)
		return
	}
	writeJSON(response, http.StatusOK, customer)
}

func (api *API) createContactHandler(response http.ResponseWriter, request *http.Request) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	var input domain.ContactInput
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	contact, err := api.service.CreateContact(request.Context(), request.PathValue("customerID"), input, operator.ID)
	if err != nil {
		api.writeBusinessError(response, "CONTACT_CREATE_FAILED", "新增联系人失败", err)
		return
	}
	writeJSON(response, http.StatusCreated, contact)
}

func (api *API) upsertBillingProfileHandler(response http.ResponseWriter, request *http.Request) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	var input domain.BillingProfileInput
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	profile, err := api.service.UpsertBillingProfile(request.Context(), request.PathValue("customerID"), input, operator.ID)
	if err != nil {
		api.writeBusinessError(response, "BILLING_PROFILE_UPDATE_FAILED", "保存开票资料失败", err)
		return
	}
	writeJSON(response, http.StatusOK, profile)
}

func (api *API) listPlansHandler(response http.ResponseWriter, request *http.Request) {
	if _, ok := api.authenticate(response, request); !ok {
		return
	}
	plans, err := api.service.ListPlans(request.Context(), queryLimit(request, 50, 100))
	if err != nil {
		api.writeBusinessError(response, "PLANS_LIST_FAILED", "读取套餐失败", err)
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{"items": plans})
}

func (api *API) createPlanHandler(response http.ResponseWriter, request *http.Request) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	var input domain.PlanInput
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	plan, err := api.service.CreatePlan(request.Context(), input, operator.ID)
	if err != nil {
		api.writeBusinessError(response, "PLAN_CREATE_FAILED", "创建套餐失败", err)
		return
	}
	writeJSON(response, http.StatusCreated, plan)
}

func (api *API) publishPlanPriceHandler(response http.ResponseWriter, request *http.Request) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	var input domain.PriceVersionInput
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	price, err := api.service.PublishPlanPrice(request.Context(), request.PathValue("planID"), input, operator.ID)
	if err != nil {
		api.writeBusinessError(response, "PLAN_PRICE_PUBLISH_FAILED", "发布价格版本失败", err)
		return
	}
	writeJSON(response, http.StatusCreated, price)
}

func (api *API) listOrdersHandler(response http.ResponseWriter, request *http.Request) {
	if _, ok := api.authenticate(response, request); !ok {
		return
	}
	orders, err := api.service.ListOrders(request.Context(), queryLimit(request, 50, 100))
	if err != nil {
		api.writeBusinessError(response, "ORDERS_LIST_FAILED", "读取订单失败", err)
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{"items": orders})
}

func (api *API) createOrderHandler(response http.ResponseWriter, request *http.Request) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	var input domain.OrderInput
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	order, err := api.service.CreateOrder(request.Context(), input, operator.ID)
	if err != nil {
		api.writeBusinessError(response, "ORDER_CREATE_FAILED", "创建订单失败", err)
		return
	}
	writeJSON(response, http.StatusCreated, order)
}

func (api *API) confirmOfflinePaymentHandler(response http.ResponseWriter, request *http.Request) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	var input domain.OfflinePaymentInput
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	if err := api.service.Reauthenticate(request.Context(), operator, input.CurrentPassword); err != nil {
		writeError(response, http.StatusUnauthorized, "REAUTHENTICATION_FAILED", "当前密码验证失败")
		return
	}
	input.CurrentPassword = ""
	order, err := api.service.ConfirmOfflinePayment(request.Context(), request.PathValue("orderID"), input, operator.ID)
	if err != nil {
		api.writeBusinessError(response, "PAYMENT_CONFIRM_FAILED", "确认线下收款失败", err)
		return
	}
	writeJSON(response, http.StatusOK, order)
}

func (api *API) listRefundNotesHandler(response http.ResponseWriter, request *http.Request) {
	if _, ok := api.authenticate(response, request); !ok {
		return
	}
	items, err := api.service.ListRefundNotes(request.Context(), request.PathValue("orderID"))
	if err != nil {
		api.writeBusinessError(response, "REFUND_NOTES_LIST_FAILED", "读取退款记录失败", err)
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{"items": items})
}

func (api *API) createRefundNoteHandler(response http.ResponseWriter, request *http.Request) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	var input domain.RefundNoteInput
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	if err := api.service.Reauthenticate(request.Context(), operator, input.CurrentPassword); err != nil {
		writeError(response, http.StatusUnauthorized, "REAUTHENTICATION_FAILED", "当前密码验证失败")
		return
	}
	input.CurrentPassword = ""
	note, err := api.service.CreateRefundNote(request.Context(), request.PathValue("orderID"), input, operator.ID)
	if err != nil {
		api.writeBusinessError(response, "REFUND_NOTE_CREATE_FAILED", "记录退款失败", err)
		return
	}
	writeJSON(response, http.StatusCreated, note)
}

func (api *API) listTrialsHandler(response http.ResponseWriter, request *http.Request) {
	if _, ok := api.authenticate(response, request); !ok {
		return
	}
	trials, err := api.service.ListTrials(request.Context(), queryLimit(request, 50, 100))
	if err != nil {
		api.writeBusinessError(response, "TRIALS_LIST_FAILED", "读取试用失败", err)
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{"items": trials})
}

func (api *API) createTrialHandler(response http.ResponseWriter, request *http.Request) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	var input domain.TrialInput
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	trial, err := api.service.CreateTrial(request.Context(), input, operator.ID)
	if err != nil {
		api.writeBusinessError(response, "TRIAL_CREATE_FAILED", "创建试用失败", err)
		return
	}
	writeJSON(response, http.StatusCreated, trial)
}

func (api *API) extendTrialHandler(response http.ResponseWriter, request *http.Request) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	var input domain.TrialExtensionInput
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	if err := api.service.Reauthenticate(request.Context(), operator, input.CurrentPassword); err != nil {
		writeError(response, http.StatusUnauthorized, "REAUTHENTICATION_FAILED", "当前密码验证失败")
		return
	}
	input.CurrentPassword = ""
	trial, err := api.service.ExtendTrial(request.Context(), request.PathValue("trialID"), input, operator.ID)
	if err != nil {
		api.writeBusinessError(response, "TRIAL_EXTENSION_FAILED", "试用延期失败", err)
		return
	}
	writeJSON(response, http.StatusOK, trial)
}

func (api *API) listRiskNotesHandler(response http.ResponseWriter, request *http.Request) {
	if _, ok := api.authenticate(response, request); !ok {
		return
	}
	items, err := api.service.ListRiskNotes(request.Context(), request.PathValue("trialID"))
	if err != nil {
		api.writeBusinessError(response, "RISK_NOTES_LIST_FAILED", "读取风险记录失败", err)
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{"items": items})
}

func (api *API) createRiskNoteHandler(response http.ResponseWriter, request *http.Request) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	var input domain.RiskNoteInput
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	note, err := api.service.CreateRiskNote(request.Context(), request.PathValue("trialID"), input, operator.ID)
	if err != nil {
		api.writeBusinessError(response, "RISK_NOTE_CREATE_FAILED", "记录试用风险失败", err)
		return
	}
	writeJSON(response, http.StatusCreated, note)
}

func (api *API) listBackupHistoryHandler(response http.ResponseWriter, request *http.Request) {
	if _, ok := api.authenticate(response, request); !ok {
		return
	}
	items, err := api.service.ListBackupHistory(request.Context(), queryLimit(request, 50, 100))
	if err != nil {
		api.writeBusinessError(response, "BACKUP_HISTORY_FAILED", "读取备份历史失败", err)
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{"items": items})
}

func (api *API) exportOperationsHandler(response http.ResponseWriter, request *http.Request) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	var input struct {
		CurrentPassword string `json:"current_password"`
	}
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	if err := api.service.Reauthenticate(request.Context(), operator, input.CurrentPassword); err != nil {
		writeError(response, http.StatusUnauthorized, "REAUTHENTICATION_FAILED", "当前密码验证失败")
		return
	}
	export, err := api.service.ExportOperations(request.Context(), operator.ID)
	if err != nil {
		api.writeBusinessError(response, "OPERATIONS_EXPORT_FAILED", "导出运营数据失败", err)
		return
	}
	response.Header().Set("Content-Disposition", `attachment; filename="aster-operations-export.json"`)
	writeJSON(response, http.StatusOK, export)
}

func (api *API) fulfillOrderHandler(response http.ResponseWriter, request *http.Request) {
	api.fulfillSourceHandler(response, request, "order")
}

func (api *API) fulfillTrialHandler(response http.ResponseWriter, request *http.Request) {
	api.fulfillSourceHandler(response, request, "trial")
}

func (api *API) fulfillSourceHandler(response http.ResponseWriter, request *http.Request, sourceType string) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	var input domain.FulfillmentInput
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	var license ports.LicenseRecord
	var err error
	if sourceType == "order" {
		license, err = api.service.FulfillOrder(request.Context(), request.PathValue("orderID"), input, operator.ID)
	} else {
		license, err = api.service.FulfillTrial(request.Context(), request.PathValue("trialID"), input, operator.ID)
	}
	if err != nil {
		api.logger.Error("create license policy failed", "operator_id", operator.ID, "source_type", sourceType, "operation_id", input.OperationID, "error", err)
		if errors.Is(err, application.ErrValidation) {
			writeError(response, http.StatusBadRequest, "FULFILLMENT_INVALID", err.Error())
			return
		}
		api.writeBusinessError(response, "LICENSE_POLICY_CREATE_FAILED", "创建许可证策略失败", err)
		return
	}
	writeJSON(response, http.StatusCreated, license)
}

func (api *API) listAuditEventsHandler(response http.ResponseWriter, request *http.Request) {
	if _, ok := api.authenticate(response, request); !ok {
		return
	}
	events, err := api.service.ListAuditEvents(request.Context(), queryLimit(request, 100, 200))
	if err != nil {
		api.writeBusinessError(response, "AUDIT_LIST_FAILED", "读取审计事件失败", err)
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{"items": events})
}

func (api *API) listReleaseArtifactsHandler(response http.ResponseWriter, request *http.Request) {
	if _, ok := api.authenticate(response, request); !ok {
		return
	}
	items, err := api.service.ListReleaseArtifacts(request.Context(), queryLimit(request, 50, 100))
	if err != nil {
		api.writeBusinessError(response, "RELEASES_LIST_FAILED", "读取发布物失败", err)
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{"items": items})
}

func (api *API) importReleaseArtifactHandler(response http.ResponseWriter, request *http.Request) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	var input domain.ReleaseArtifactInput
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	item, err := api.service.ImportReleaseArtifact(request.Context(), input, operator.ID)
	if err != nil {
		api.writeBusinessError(response, "RELEASE_IMPORT_FAILED", "导入发布物失败", err)
		return
	}
	writeJSON(response, http.StatusCreated, item)
}

func (api *API) downloadReleaseArtifactHandler(response http.ResponseWriter, request *http.Request) {
	if _, ok := api.authorizePermission(response, request, application.PermissionReleaseDownload); !ok {
		return
	}
	release, reader, err := api.service.OpenReleaseArtifact(request.Context(), request.PathValue("artifactID"))
	if err != nil {
		api.writeBusinessError(response, "RELEASE_DOWNLOAD_FAILED", "下载安装包失败", err)
		return
	}
	defer reader.Close()
	filename := release.FileName()
	response.Header().Set("Content-Type", "application/gzip")
	response.Header().Set("Content-Disposition", `attachment; filename="`+filename+`"`)
	response.Header().Set("Content-Length", strconv.FormatInt(release.SizeBytes, 10))
	response.Header().Set("ETag", `"sha256:`+release.SHA256+`"`)
	response.WriteHeader(http.StatusOK)
	if _, err := io.Copy(response, reader); err != nil {
		api.logger.Warn("stream release artifact failed", "release_artifact_id", release.ID, "error", err)
	}
}

func (api *API) listReleaseTasksHandler(response http.ResponseWriter, request *http.Request) {
	if _, ok := api.authorizePermission(response, request, application.PermissionReleaseRead); !ok {
		return
	}
	items, err := api.service.ListReleaseTasks(request.Context(), queryLimit(request, 50, 100))
	if err != nil {
		api.writeBusinessError(response, "RELEASE_TASKS_LIST_FAILED", "读取发布任务失败", err)
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{"items": items})
}

func (api *API) getReleaseTaskHandler(response http.ResponseWriter, request *http.Request) {
	if _, ok := api.authorizePermission(response, request, application.PermissionReleaseRead); !ok {
		return
	}
	detail, err := api.service.GetReleaseTaskDetail(request.Context(), request.PathValue("taskID"))
	if err != nil {
		api.writeBusinessError(response, "RELEASE_TASK_DETAIL_FAILED", "读取发布任务详情失败", err)
		return
	}
	writeJSON(response, http.StatusOK, detail)
}

func (api *API) releaseCapabilitiesHandler(response http.ResponseWriter, request *http.Request) {
	if _, ok := api.authorizePermission(response, request, application.PermissionReleaseRead); !ok {
		return
	}
	writeJSON(response, http.StatusOK, api.service.ReleaseCapabilities())
}

func (api *API) createReleaseTaskHandler(response http.ResponseWriter, request *http.Request) {
	operator, ok := api.authorizeMutationPermission(response, request, application.PermissionReleaseBuild)
	if !ok {
		return
	}
	var input domain.ReleaseTaskInput
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	detail, err := api.service.CreateReleaseTask(request.Context(), input, operator.ID)
	if err != nil {
		api.writeReleaseMutationError(response, "RELEASE_TASK_CREATE_FAILED", "创建发布任务失败", err)
		return
	}
	writeJSON(response, http.StatusCreated, detail)
}

func (api *API) syncReleaseTaskHandler(response http.ResponseWriter, request *http.Request) {
	if _, ok := api.authorizeMutationPermission(response, request, application.PermissionReleaseBuild); !ok {
		return
	}
	taskID := request.PathValue("taskID")
	if err := api.service.SyncReleaseTask(request.Context(), taskID); err != nil {
		api.writeReleaseMutationError(response, "GITHUB_SYNC_FAILED", "GitHub Run 状态同步失败", err)
		return
	}
	detail, err := api.service.GetReleaseTaskDetail(request.Context(), taskID)
	if err != nil {
		api.writeBusinessError(response, "RELEASE_TASK_DETAIL_FAILED", "读取发布任务详情失败", err)
		return
	}
	writeJSON(response, http.StatusOK, detail)
}

func (api *API) retryReleaseTaskHandler(response http.ResponseWriter, request *http.Request) {
	operator, ok := api.authorizeMutationPermission(response, request, application.PermissionReleaseBuild)
	if !ok {
		return
	}
	detail, err := api.service.RetryReleaseTask(request.Context(), request.PathValue("taskID"), operator.ID)
	if err != nil {
		api.writeReleaseMutationError(response, "RELEASE_RETRY_FAILED", "重试发布任务失败", err)
		return
	}
	writeJSON(response, http.StatusCreated, detail)
}

func (api *API) reverifyReleaseTaskHandler(response http.ResponseWriter, request *http.Request) {
	operator, ok := api.authorizeMutationPermission(response, request, application.PermissionReleaseBuild)
	if !ok {
		return
	}
	detail, err := api.service.ReverifyReleaseTask(request.Context(), request.PathValue("taskID"), request.PathValue("artifactID"), operator.ID)
	if err != nil {
		api.writeReleaseMutationError(response, "RELEASE_REVERIFY_FAILED", "重新复验发布产物失败", err)
		return
	}
	writeJSON(response, http.StatusAccepted, detail)
}

func (api *API) listReleasePublishRequestsHandler(response http.ResponseWriter, request *http.Request) {
	if _, ok := api.authorizePermission(response, request, application.PermissionReleaseRead); !ok {
		return
	}
	items, err := api.service.ListReleasePublishRequests(request.Context(), queryLimit(request, 50, 100))
	if err != nil {
		api.writeBusinessError(response, "RELEASE_PUBLISH_REQUESTS_LIST_FAILED", "读取正式发布申请失败", err)
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{"items": items})
}

func (api *API) requestReleasePublishHandler(response http.ResponseWriter, request *http.Request) {
	operator, ok := api.authorizeMutationPermission(response, request, application.PermissionReleasePublishRequest)
	if !ok {
		return
	}
	var input struct{}
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	item, err := api.service.RequestReleasePublish(request.Context(), request.PathValue("artifactID"), operator.ID)
	if err != nil {
		api.writeReleasePublishError(response, "RELEASE_PUBLISH_REQUEST_FAILED", "创建正式发布申请失败", err)
		return
	}
	writeJSON(response, http.StatusCreated, item)
}

func (api *API) decideReleasePublishHandler(response http.ResponseWriter, request *http.Request) {
	operator, ok := api.authorizeMutationPermission(response, request, application.PermissionReleasePublishApprove)
	if !ok {
		return
	}
	var input struct {
		Decision        string `json:"decision"`
		Comment         string `json:"comment"`
		CurrentPassword string `json:"current_password"`
	}
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	if err := api.service.Reauthenticate(request.Context(), operator, input.CurrentPassword); err != nil {
		writeError(response, http.StatusUnauthorized, "REAUTHENTICATION_FAILED", "当前密码验证失败")
		return
	}
	item, err := api.service.DecideReleasePublish(request.Context(), request.PathValue("requestID"),
		domain.ReleasePublishApprovalInput{Decision: input.Decision, Comment: input.Comment}, operator.ID)
	if err != nil {
		api.writeReleasePublishError(response, "RELEASE_PUBLISH_APPROVAL_FAILED", "正式发布审批失败", err)
		return
	}
	writeJSON(response, http.StatusOK, item)
}

func (api *API) executeReleasePublishHandler(response http.ResponseWriter, request *http.Request) {
	operator, ok := api.authorizeMutationPermission(response, request, application.PermissionReleasePublishExecute)
	if !ok {
		return
	}
	var input struct {
		CurrentPassword string `json:"current_password"`
	}
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	if err := api.service.Reauthenticate(request.Context(), operator, input.CurrentPassword); err != nil {
		writeError(response, http.StatusUnauthorized, "REAUTHENTICATION_FAILED", "当前密码验证失败")
		return
	}
	item, err := api.service.ExecuteReleasePublish(request.Context(), request.PathValue("requestID"), operator.ID)
	if err != nil {
		api.writeReleasePublishError(response, "GITHUB_PUBLISH_FAILED", "正式发布执行失败", err)
		return
	}
	writeJSON(response, http.StatusOK, item)
}

func (api *API) writeReleaseMutationError(response http.ResponseWriter, fallbackCode, fallbackMessage string, err error) {
	switch {
	case errors.Is(err, application.ErrValidation):
		writeError(response, http.StatusBadRequest, "RELEASE_TASK_INPUT_INVALID", err.Error())
	case errors.Is(err, application.ErrReleaseNotConfigured):
		writeError(response, http.StatusServiceUnavailable, "RELEASE_ORCHESTRATOR_UNAVAILABLE", "发布中心尚未配置 GitHub App")
	case errors.Is(err, application.ErrReleaseDuplicate):
		writeError(response, http.StatusConflict, "RELEASE_DUPLICATE_ACTIVE", "相同版本和提交已有未结束的发布任务")
	case errors.Is(err, application.ErrReleasePreflight):
		writeError(response, http.StatusBadGateway, "GITHUB_PREFLIGHT_FAILED", err.Error())
	case errors.Is(err, application.ErrReleaseDispatch):
		writeError(response, http.StatusBadGateway, "GITHUB_DISPATCH_FAILED", err.Error())
	case errors.Is(err, application.ErrReleaseSync):
		writeError(response, http.StatusBadGateway, "GITHUB_SYNC_FAILED", err.Error())
	default:
		api.writeBusinessError(response, fallbackCode, fallbackMessage, err)
	}
}

func (api *API) writeReleasePublishError(response http.ResponseWriter, fallbackCode, fallbackMessage string, err error) {
	switch {
	case errors.Is(err, application.ErrValidation):
		writeError(response, http.StatusBadRequest, "RELEASE_PUBLISH_INPUT_INVALID", err.Error())
	case errors.Is(err, application.ErrReleasePublishNotConfigured):
		writeError(response, http.StatusServiceUnavailable, "RELEASE_PUBLISHER_UNAVAILABLE", "正式发布尚未配置独立 GitHub App")
	case errors.Is(err, application.ErrReleasePublishDuplicate):
		writeError(response, http.StatusConflict, "RELEASE_PUBLISH_DUPLICATE", "该版本已有未结束或已完成的正式发布申请")
	case errors.Is(err, application.ErrReleasePublishApproval):
		writeError(response, http.StatusConflict, "RELEASE_PUBLISH_SELF_APPROVAL_FORBIDDEN", err.Error())
	case errors.Is(err, application.ErrReleasePublish):
		writeError(response, http.StatusBadGateway, "GITHUB_PUBLISH_FAILED", err.Error())
	default:
		api.writeBusinessError(response, fallbackCode, fallbackMessage, err)
	}
}

func (api *API) listDeliveriesHandler(response http.ResponseWriter, request *http.Request) {
	if _, ok := api.authenticate(response, request); !ok {
		return
	}
	items, err := api.service.ListDeliveries(request.Context(), queryLimit(request, 50, 100))
	if err != nil {
		api.writeBusinessError(response, "DELIVERIES_LIST_FAILED", "读取交付记录失败", err)
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{"items": items})
}

func (api *API) createDeliveryHandler(response http.ResponseWriter, request *http.Request) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	var input domain.DeliveryInput
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	item, err := api.service.CreateDelivery(request.Context(), input, operator.ID)
	if err != nil {
		api.writeBusinessError(response, "DELIVERY_CREATE_FAILED", "创建交付记录失败", err)
		return
	}
	writeJSON(response, http.StatusCreated, item)
}

func (api *API) deliveryReceiptHandler(response http.ResponseWriter, request *http.Request) {
	if _, ok := api.authenticate(response, request); !ok {
		return
	}
	contents, delivery, err := api.service.GetDeliveryReceipt(request.Context(), request.PathValue("deliveryID"))
	if err != nil {
		api.writeBusinessError(response, "DELIVERY_RECEIPT_FAILED", "读取交付回执失败", err)
		return
	}
	response.Header().Set("Content-Type", "application/json; charset=utf-8")
	response.Header().Set("Content-Disposition", `attachment; filename="`+delivery.ID+`-receipt.json"`)
	response.Header().Set("Cache-Control", "no-store")
	response.WriteHeader(http.StatusOK)
	_, _ = response.Write(contents)
}

func (api *API) writeBusinessError(response http.ResponseWriter, code, message string, err error) {
	if errors.Is(err, application.ErrValidation) {
		writeError(response, http.StatusBadRequest, "VALIDATION_FAILED", err.Error())
		return
	}
	if errors.Is(err, application.ErrNotFound) {
		writeError(response, http.StatusNotFound, "NOT_FOUND", "资源不存在")
		return
	}
	if errors.Is(err, application.ErrBusinessStoreUnavailable) {
		writeError(response, http.StatusServiceUnavailable, "BUSINESS_STORE_UNAVAILABLE", "运营业务数据库不可用")
		return
	}
	api.logger.Error("operations business request failed", "code", code, "error", err)
	writeError(response, http.StatusInternalServerError, code, message)
}

func queryLimit(request *http.Request, fallback, maximum int) int {
	raw := request.URL.Query().Get("limit")
	if raw == "" {
		return fallback
	}
	value, err := strconv.Atoi(raw)
	if err != nil || value < 1 || value > maximum {
		return fallback
	}
	return value
}

func (api *API) listLicensesHandler(response http.ResponseWriter, request *http.Request) {
	if _, ok := api.authenticate(response, request); !ok {
		return
	}
	limit := 50
	if raw := request.URL.Query().Get("limit"); raw != "" {
		parsed, err := strconv.Atoi(raw)
		if err != nil || parsed < 1 || parsed > 100 {
			writeError(response, http.StatusBadRequest, "INVALID_LIMIT", "limit 必须在 1 到 100 之间")
			return
		}
		limit = parsed
	}
	licenses, err := api.service.ListLicenseRecords(request.Context(), limit)
	if err != nil {
		api.writeBusinessError(response, "LICENSES_QUERY_FAILED", "读取许可证策略失败", err)
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{"items": licenses})
}

func (api *API) authenticate(response http.ResponseWriter, request *http.Request) (domain.AuthenticatedOperator, bool) {
	cookie, err := request.Cookie(sessionCookieName)
	if err != nil {
		writeError(response, http.StatusUnauthorized, "UNAUTHORIZED", "请先登录")
		return domain.AuthenticatedOperator{}, false
	}
	operator, err := api.service.Authenticate(request.Context(), cookie.Value)
	if err != nil {
		api.clearSessionCookies(response)
		writeError(response, http.StatusUnauthorized, "UNAUTHORIZED", "会话无效或已过期")
		return domain.AuthenticatedOperator{}, false
	}
	return operator, true
}

func (api *API) authorizePermission(response http.ResponseWriter, request *http.Request, permission string) (domain.AuthenticatedOperator, bool) {
	operator, ok := api.authenticate(response, request)
	if !ok {
		return domain.AuthenticatedOperator{}, false
	}
	if err := api.service.RequirePermission(request.Context(), operator.ID, permission); err != nil {
		if errors.Is(err, application.ErrUnauthorized) {
			writeError(response, http.StatusForbidden, "RELEASE_PERMISSION_DENIED", "当前运营账号没有发布中心查看权限")
			return domain.AuthenticatedOperator{}, false
		}
		api.writeBusinessError(response, "RELEASE_PERMISSION_CHECK_FAILED", "发布中心权限检查失败", err)
		return domain.AuthenticatedOperator{}, false
	}
	return operator, true
}

func (api *API) authorizeMutationPermission(response http.ResponseWriter, request *http.Request, permission string) (domain.AuthenticatedOperator, bool) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return domain.AuthenticatedOperator{}, false
	}
	if err := api.service.RequirePermission(request.Context(), operator.ID, permission); err != nil {
		if errors.Is(err, application.ErrUnauthorized) {
			writeError(response, http.StatusForbidden, "RELEASE_PERMISSION_DENIED", "当前运营账号没有发布中心操作权限")
			return domain.AuthenticatedOperator{}, false
		}
		api.writeBusinessError(response, "RELEASE_PERMISSION_CHECK_FAILED", "发布中心权限检查失败", err)
		return domain.AuthenticatedOperator{}, false
	}
	return operator, true
}

func (api *API) authorizeMutation(response http.ResponseWriter, request *http.Request) (domain.AuthenticatedOperator, string, bool) {
	if !api.validOrigin(request) {
		writeError(response, http.StatusForbidden, "ORIGIN_FORBIDDEN", "请求来源不受信任")
		return domain.AuthenticatedOperator{}, "", false
	}
	sessionCookie, err := request.Cookie(sessionCookieName)
	if err != nil {
		writeError(response, http.StatusUnauthorized, "UNAUTHORIZED", "请先登录")
		return domain.AuthenticatedOperator{}, "", false
	}
	operator, err := api.service.Authenticate(request.Context(), sessionCookie.Value)
	if err != nil {
		api.clearSessionCookies(response)
		writeError(response, http.StatusUnauthorized, "UNAUTHORIZED", "会话无效或已过期")
		return domain.AuthenticatedOperator{}, "", false
	}
	csrfCookie, err := request.Cookie(csrfCookieName)
	if err != nil || !api.service.ValidateCSRF(operator, csrfCookie.Value, request.Header.Get("X-CSRF-Token")) {
		writeError(response, http.StatusForbidden, "CSRF_INVALID", "CSRF 校验失败")
		return domain.AuthenticatedOperator{}, "", false
	}
	return operator, sessionCookie.Value, true
}

func (api *API) validOrigin(request *http.Request) bool {
	origin := strings.TrimRight(strings.TrimSpace(request.Header.Get("Origin")), "/")
	_, ok := api.config.TrustedOrigins[origin]
	return ok
}

func (api *API) decodeJSON(response http.ResponseWriter, request *http.Request, destination any) error {
	mediaType, _, err := mime.ParseMediaType(request.Header.Get("Content-Type"))
	if err != nil || mediaType != "application/json" {
		return errors.New("Content-Type 必须是 application/json")
	}
	request.Body = http.MaxBytesReader(response, request.Body, api.config.BodyMaxSize)
	decoder := json.NewDecoder(request.Body)
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(destination); err != nil {
		return errors.New("请求 JSON 无效")
	}
	if err := decoder.Decode(&struct{}{}); !errors.Is(err, io.EOF) {
		return errors.New("请求只能包含一个 JSON 对象")
	}
	return nil
}

func (api *API) setSessionCookies(response http.ResponseWriter, tokens domain.SessionTokens) {
	maxAge := int(time.Until(tokens.ExpiresAt).Seconds())
	http.SetCookie(response, &http.Cookie{Name: sessionCookieName, Value: tokens.SessionToken, Path: "/", HttpOnly: true, Secure: api.config.CookieSecure, SameSite: http.SameSiteStrictMode, MaxAge: maxAge, Expires: tokens.ExpiresAt})
	http.SetCookie(response, &http.Cookie{Name: csrfCookieName, Value: tokens.CSRFToken, Path: "/", HttpOnly: false, Secure: api.config.CookieSecure, SameSite: http.SameSiteStrictMode, MaxAge: maxAge, Expires: tokens.ExpiresAt})
}

func (api *API) clearSessionCookies(response http.ResponseWriter) {
	expired := time.Unix(1, 0)
	http.SetCookie(response, &http.Cookie{Name: sessionCookieName, Value: "", Path: "/", HttpOnly: true, Secure: api.config.CookieSecure, SameSite: http.SameSiteStrictMode, MaxAge: -1, Expires: expired})
	http.SetCookie(response, &http.Cookie{Name: csrfCookieName, Value: "", Path: "/", HttpOnly: false, Secure: api.config.CookieSecure, SameSite: http.SameSiteStrictMode, MaxAge: -1, Expires: expired})
}

func writeJSON(response http.ResponseWriter, status int, body any) {
	response.Header().Set("Content-Type", "application/json; charset=utf-8")
	response.WriteHeader(status)
	_ = json.NewEncoder(response).Encode(body)
}

func writeError(response http.ResponseWriter, status int, code, message string) int64 {
	return apierrors.Write(response, status, code, message)
}

func randomRequestID() string {
	value := make([]byte, 8)
	if _, err := rand.Read(value); err != nil {
		return "request"
	}
	return hex.EncodeToString(value)
}
