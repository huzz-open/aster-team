package httpapi

import (
	"errors"
	"net/http"
	"strconv"

	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/commercial"
)

func (api *API) commercialPlansHandler(response http.ResponseWriter, request *http.Request) {
	operator, ok := api.authenticate(response, request)
	if !ok {
		return
	}
	items, err := api.service.ListCommercialPlans(request.Context(), queryLimit(request, 50, 100), operator.ID)
	if err != nil {
		api.writeCommercialError(response, "PLANS_LIST_FAILED", "读取套餐版本失败", err)
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{"items": items})
}
func (api *API) freezeCommercialPlanHandler(response http.ResponseWriter, request *http.Request) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	var input application.FreezeCommercialPlanInput
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	record, err := api.service.FreezeCommercialPlan(request.Context(), input, operator.ID)
	if err != nil {
		api.writeCommercialError(response, "PLAN_CREATE_FAILED", "保存套餐版本失败", err)
		return
	}
	writeJSON(response, http.StatusCreated, record)
}
func (api *API) getCommercialPlanHandler(response http.ResponseWriter, request *http.Request) {
	operator, ok := api.authenticate(response, request)
	if !ok {
		return
	}
	version, err := strconv.ParseUint(request.PathValue("version"), 10, 32)
	if err != nil || version == 0 {
		writeError(response, http.StatusBadRequest, "VALIDATION_FAILED", "套餐版本无效")
		return
	}
	record, err := api.service.GetCommercialPlan(request.Context(), request.PathValue("planID"), uint32(version), operator.ID)
	if err != nil {
		api.writeCommercialError(response, "PLANS_LIST_FAILED", "读取套餐版本失败", err)
		return
	}
	writeJSON(response, http.StatusOK, record)
}
func (api *API) getCurrentCommercialPlanHandler(response http.ResponseWriter, request *http.Request) {
	operator, ok := api.authenticate(response, request)
	if !ok {
		return
	}
	record, err := api.service.GetCurrentCommercialPlan(request.Context(), request.PathValue("planID"), operator.ID)
	if err != nil {
		api.writeCommercialError(response, "PLANS_LIST_FAILED", "读取当前套餐版本失败", err)
		return
	}
	writeJSON(response, http.StatusOK, record)
}
func (api *API) commercialOrdersHandler(response http.ResponseWriter, request *http.Request) {
	operator, ok := api.authenticate(response, request)
	if !ok {
		return
	}
	offset := 0
	if raw := request.URL.Query().Get("offset"); raw != "" {
		value, parseErr := strconv.Atoi(raw)
		if parseErr != nil || value < 0 || value > 10_000_000 {
			writeError(response, http.StatusBadRequest, "INVALID_OFFSET", "offset 必须是 0 到 10000000 之间的整数")
			return
		}
		offset = value
	}
	page, err := api.service.ListCommercialOrders(request.Context(), application.CommercialOrderListQuery{
		Limit: queryLimit(request, 50, 100), Offset: offset, Keyword: request.URL.Query().Get("keyword"),
		Status: request.URL.Query().Get("status"), Stage: request.URL.Query().Get("stage"),
	}, operator.ID)
	if err != nil {
		api.writeCommercialError(response, "ORDERS_LIST_FAILED", "读取订单快照失败", err)
		return
	}
	writeJSON(response, http.StatusOK, page)
}
func (api *API) createCommercialOrderHandler(response http.ResponseWriter, request *http.Request) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	var input application.CommercialOrderInput
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	record, err := api.service.CreateCommercialOrder(request.Context(), input, operator.ID)
	if err != nil {
		api.writeCommercialError(response, "ORDER_CREATE_FAILED", "创建固定权益订单失败", err)
		return
	}
	writeJSON(response, http.StatusCreated, record)
}
func (api *API) getCommercialOrderHandler(response http.ResponseWriter, request *http.Request) {
	operator, ok := api.authenticate(response, request)
	if !ok {
		return
	}
	record, err := api.service.GetCommercialOrder(request.Context(), request.PathValue("orderID"), operator.ID)
	if err != nil {
		api.writeCommercialError(response, "ORDERS_LIST_FAILED", "读取订单快照失败", err)
		return
	}
	writeJSON(response, http.StatusOK, record)
}

func (api *API) createQuotationOrderHandler(response http.ResponseWriter, request *http.Request) {
	operator, _, ok := api.authorizeMutation(response, request)
	if !ok {
		return
	}
	var input commercial.QuotationOrderInput
	if err := api.decodeJSON(response, request, &input); err != nil {
		writeError(response, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	record, err := api.service.CreateQuotationOrder(request.Context(), input, operator.ID)
	if err != nil {
		api.writeCommercialError(response, "ORDER_CREATE_FAILED", "创建官网报价订单失败", err)
		return
	}
	writeJSON(response, http.StatusCreated, record)
}

func (api *API) getQuotationSourceHandler(response http.ResponseWriter, request *http.Request) {
	operator, ok := api.authenticate(response, request)
	if !ok {
		return
	}
	source, err := api.service.GetQuotationSource(request.Context(), request.PathValue("reference"), operator.ID)
	if err != nil {
		api.writeCommercialError(response, "ORDERS_LIST_FAILED", "没有找到当前受理环境内可用的官网报价来源", err)
		return
	}
	writeJSON(response, http.StatusOK, source)
}

func (api *API) writeCommercialError(response http.ResponseWriter, code, message string, err error) {
	if errors.Is(err, application.ErrV2VerifierUnavailable) {
		writeError(response, http.StatusServiceUnavailable, "COMMERCIAL_V2_VERIFIER_UNAVAILABLE", "缺少该证书的可信公钥，无法核验原授权文件")
		return
	}
	if errors.Is(err, commercial.ErrCustomerReferenceUnavailable) {
		writeError(response, http.StatusServiceUnavailable, "COMMERCIAL_CUSTOMER_REF_UNAVAILABLE", "客户引用派生能力不可用，新履约尚未批准")
		return
	}
	if errors.Is(err, commercial.ErrFulfillmentEnvironment) {
		writeError(response, http.StatusConflict, "COMMERCIAL_FULFILLMENT_ENVIRONMENT", "当前履约环境未配置或与原批准来源不同")
		return
	}
	if errors.Is(err, application.ErrPublicationFailureRecordUnavailable) {
		writeError(response, http.StatusServiceUnavailable, "COMMERCIAL_PUBLICATION_FAILURE_NOT_RECORDED", "未能保存本次核对的故障记录，请重新读取原发布记录确认结果")
		return
	}
	if errors.Is(err, commercial.ErrPublicationPreparationRejected) {
		writeError(response, http.StatusConflict, "COMMERCIAL_PUBLICATION_PREPARE_REJECTED", "原请求未保存，环境记录或受理期限已变化，请重新核对后准备")
		return
	}
	if errors.Is(err, application.ErrPublicationVerifierUnavailable) {
		writeError(response, http.StatusServiceUnavailable, "COMMERCIAL_PUBLICATION_UNAVAILABLE", "尚未配置对应环境的官网核对地址")
		return
	}
	if errors.Is(err, application.ErrPublicationVerificationFailed) {
		writeError(response, http.StatusConflict, "COMMERCIAL_PUBLICATION_UNVERIFIED", "官网内容尚未通过核对，请保留原发布记录并在排查后重试")
		return
	}
	if errors.Is(err, application.ErrCatalogExporterUnavailable) {
		writeError(response, http.StatusServiceUnavailable, "COMMERCIAL_CATALOG_EXPORT_UNAVAILABLE", "尚未配置公开目录导出位置")
		return
	}
	if errors.Is(err, application.ErrCatalogNotExported) {
		writeError(response, http.StatusConflict, "COMMERCIAL_CATALOG_NOT_EXPORTED", "请先将该批准目录导出到运营主机")
		return
	}
	if errors.Is(err, commercial.ErrCatalogPreviewConflict) {
		writeError(response, http.StatusConflict, "COMMERCIAL_CATALOG_PREVIEW_CONFLICT", "本次批准未保存，公开内容已与预览不同，请重新预览后核对")
		return
	}
	if errors.Is(err, application.ErrV2SignerUnavailable) {
		writeError(response, http.StatusServiceUnavailable, "COMMERCIAL_V2_SIGNER_UNAVAILABLE", "尚未配置对应的受限 v2 签发密钥")
		return
	}
	if errors.Is(err, application.ErrUnauthorized) {
		writeError(response, http.StatusForbidden, "COMMERCIAL_PERMISSION_DENIED", "当前运营账号没有此项套餐或订单管理权限")
		return
	}
	if errors.Is(err, commercial.ErrDraftRevisionConflict) {
		writeError(response, http.StatusConflict, "COMMERCIAL_DRAFT_REVISION_CONFLICT", "本次操作未保存且草稿已变化，请比较最新修订后继续")
		return
	}
	if errors.Is(err, commercial.ErrPlanVersionConflict) {
		writeError(response, http.StatusConflict, "COMMERCIAL_PLAN_VERSION_CONFLICT", "本次操作未保存且套餐版本已变化，请比较最新版本后继续")
		return
	}
	if errors.Is(err, commercial.ErrConflict) {
		writeError(response, http.StatusConflict, "COMMERCIAL_SNAPSHOT_CONFLICT", "套餐版本已变化或操作标识对应其他内容，请核对最新版本")
		return
	}
	api.writeBusinessError(response, code, message, err)
}
