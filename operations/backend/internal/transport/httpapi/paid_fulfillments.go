package httpapi

import (
	"encoding/json"
	"fmt"
	"net/http"
	"strconv"

	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/strictjson"
)

type paidApprovalFields commercial.ApprovePaidFulfillmentInput
type approvePaidInput struct {
	paidApprovalFields
	CurrentPassword string `json:"current_password"`
}

type paidRedeliveryFields commercial.RecordPaidRedeliveryInput
type recordPaidRedeliveryInput struct {
	paidRedeliveryFields
	CurrentPassword string `json:"current_password"`
}

type paidTransferFields commercial.ApprovePaidTransferInput
type approvePaidTransferInput struct {
	paidTransferFields
	CurrentPassword string `json:"current_password"`
}

func (v *approvePaidTransferInput) UnmarshalJSON(data []byte) error {
	type wire approvePaidTransferInput
	var result wire
	if err := strictjson.Object(data, &result, "operation_id", "expected_current_document_sha256", "license_request_json", "reason", "current_password"); err != nil {
		return err
	}
	*v = approvePaidTransferInput(result)
	return nil
}

func (v *recordPaidRedeliveryInput) UnmarshalJSON(data []byte) error {
	type wire recordPaidRedeliveryInput
	var result wire
	if err := strictjson.Object(data, &result, "operation_id", "expected_document_sha256", "reason", "current_password"); err != nil {
		return err
	}
	*v = recordPaidRedeliveryInput(result)
	return nil
}

func (v *approvePaidInput) UnmarshalJSON(data []byte) error {
	type wire approvePaidInput
	var result wire
	fields := []string{"operation_id", "expected_order_sha256", "expected_payment_sha256", "license_request_json", "reason", "current_password"}
	var shape map[string]json.RawMessage
	if err := json.Unmarshal(data, &shape); err == nil {
		if _, ok := shape["lifecycle"]; ok {
			fields = append(fields, "lifecycle")
		}
	}
	if err := strictjson.Object(data, &result, fields...); err != nil {
		return err
	}
	*v = approvePaidInput(result)
	return nil
}
func (api *API) approvePaidFulfillmentHandler(w http.ResponseWriter, r *http.Request) {
	actor, _, ok := api.authorizeMutation(w, r)
	if !ok {
		return
	}
	var input approvePaidInput
	if err := api.decodeJSON(w, r, &input); err != nil {
		writeError(w, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	if err := api.service.Reauthenticate(r.Context(), actor, input.CurrentPassword); err != nil {
		writeError(w, http.StatusUnauthorized, "REAUTHENTICATION_FAILED", "当前密码验证失败")
		return
	}
	input.CurrentPassword = ""
	record, err := api.service.ApprovePaidFulfillment(r.Context(), r.PathValue("orderID"), commercial.ApprovePaidFulfillmentInput(input.paidApprovalFields), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_FULFILLMENT_FAILED", "批准付费履约失败", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}
func (api *API) paidFulfillmentContextHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	record, err := api.service.GetPaidFulfillmentContext(r.Context(), r.PathValue("orderID"), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_FULFILLMENT_FAILED", "读取付费履约核对信息失败", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}
func (api *API) paidFulfillmentForOrderHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	record, err := api.service.GetPaidFulfillmentForOrder(r.Context(), r.PathValue("orderID"), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_FULFILLMENT_FAILED", "读取订单履约记录失败", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}
func (api *API) getPaidLifecycleSourceHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	record, err := api.service.GetPaidLifecycleSource(r.Context(), r.PathValue("kind"), r.PathValue("sourceID"), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_LIFECYCLE_SOURCE_FAILED", "读取续费、升级或试用转换来源失败", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}
func (api *API) getPaidFulfillmentHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	record, err := api.service.GetPaidFulfillment(r.Context(), r.PathValue("fulfillmentID"), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_FULFILLMENT_FAILED", "读取付费履约记录失败", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}
func (api *API) issuePaidFulfillmentHandler(w http.ResponseWriter, r *http.Request) {
	actor, _, ok := api.authorizeMutation(w, r)
	if !ok {
		return
	}
	var input issueDistributionInput
	if err := api.decodeJSON(w, r, &input); err != nil {
		writeError(w, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	if err := api.service.Reauthenticate(r.Context(), actor, input.CurrentPassword); err != nil {
		writeError(w, http.StatusUnauthorized, "REAUTHENTICATION_FAILED", "当前密码验证失败")
		return
	}
	input.CurrentPassword = ""
	record, err := api.service.IssuePaidFulfillment(r.Context(), r.PathValue("fulfillmentID"), input.KeyID, actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_FULFILLMENT_FAILED", "签发付费授权失败", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}
func (api *API) downloadPaidFulfillmentHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	record, err := api.service.GetPaidFulfillment(r.Context(), r.PathValue("fulfillmentID"), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_FULFILLMENT_FAILED", "核验付费授权文件失败", err)
		return
	}
	if record.Status != "issued" || record.Document == nil {
		writeError(w, http.StatusConflict, "COMMERCIAL_FULFILLMENT_NOT_ISSUED", "该付费履约尚未完成签发")
		return
	}
	raw, err := json.Marshal(record.Document)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_FULFILLMENT_FAILED", "读取付费授权文件失败", err)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Content-Length", strconv.Itoa(len(raw)))
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("Content-Disposition", fmt.Sprintf("attachment; filename=\"aster-paid-%s.license.json\"", record.Snapshot.ID))
	w.Header().Set("X-Content-SHA256", record.DocumentSHA256)
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write(raw)
}

func (api *API) recordPaidRedeliveryHandler(w http.ResponseWriter, r *http.Request) {
	actor, _, ok := api.authorizeMutation(w, r)
	if !ok {
		return
	}
	var input recordPaidRedeliveryInput
	if err := api.decodeJSON(w, r, &input); err != nil {
		writeError(w, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	if err := api.service.Reauthenticate(r.Context(), actor, input.CurrentPassword); err != nil {
		writeError(w, http.StatusUnauthorized, "REAUTHENTICATION_FAILED", "当前密码验证失败")
		return
	}
	input.CurrentPassword = ""
	record, err := api.service.RecordPaidRedelivery(r.Context(), r.PathValue("fulfillmentID"), commercial.RecordPaidRedeliveryInput(input.paidRedeliveryFields), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_FULFILLMENT_REDELIVERY_FAILED", "记录付费授权补发失败", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}

func (api *API) getPaidRedeliveryHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	record, err := api.service.GetPaidRedelivery(r.Context(), r.PathValue("redeliveryID"), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_FULFILLMENT_REDELIVERY_FAILED", "读取付费授权补发记录失败", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}

func (api *API) approvePaidTransferHandler(w http.ResponseWriter, r *http.Request) {
	actor, _, ok := api.authorizeMutation(w, r)
	if !ok {
		return
	}
	var input approvePaidTransferInput
	if err := api.decodeJSON(w, r, &input); err != nil {
		writeError(w, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	if err := api.service.Reauthenticate(r.Context(), actor, input.CurrentPassword); err != nil {
		writeError(w, http.StatusUnauthorized, "REAUTHENTICATION_FAILED", "当前密码验证失败")
		return
	}
	input.CurrentPassword = ""
	record, err := api.service.ApprovePaidTransfer(r.Context(), r.PathValue("fulfillmentID"), commercial.ApprovePaidTransferInput(input.paidTransferFields), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_FULFILLMENT_TRANSFER_FAILED", "批准换机授权失败", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}

func (api *API) getPaidTransferHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	record, err := api.service.GetPaidTransfer(r.Context(), r.PathValue("transferID"), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_FULFILLMENT_TRANSFER_FAILED", "读取换机授权失败", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}

func (api *API) getLatestPaidTransferHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	record, err := api.service.GetLatestPaidTransfer(r.Context(), r.PathValue("fulfillmentID"), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_FULFILLMENT_TRANSFER_FAILED", "读取当前换机授权失败", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}

func (api *API) issuePaidTransferHandler(w http.ResponseWriter, r *http.Request) {
	actor, _, ok := api.authorizeMutation(w, r)
	if !ok {
		return
	}
	var input issueDistributionInput
	if err := api.decodeJSON(w, r, &input); err != nil {
		writeError(w, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	if err := api.service.Reauthenticate(r.Context(), actor, input.CurrentPassword); err != nil {
		writeError(w, http.StatusUnauthorized, "REAUTHENTICATION_FAILED", "当前密码验证失败")
		return
	}
	input.CurrentPassword = ""
	record, err := api.service.IssuePaidTransfer(r.Context(), r.PathValue("transferID"), input.KeyID, actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_FULFILLMENT_TRANSFER_FAILED", "签发换机授权失败", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}

func (api *API) downloadPaidTransferHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	record, err := api.service.GetPaidTransfer(r.Context(), r.PathValue("transferID"), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_FULFILLMENT_TRANSFER_FAILED", "核验换机授权失败", err)
		return
	}
	if record.Status != "issued" || record.Document == nil {
		writeError(w, http.StatusConflict, "COMMERCIAL_FULFILLMENT_TRANSFER_NOT_ISSUED", "该换机授权尚未完成签发")
		return
	}
	raw, err := json.Marshal(record.Document)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_FULFILLMENT_TRANSFER_FAILED", "读取换机授权文件失败", err)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Content-Length", strconv.Itoa(len(raw)))
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("Content-Disposition", fmt.Sprintf("attachment; filename=\"aster-paid-transfer-%s.license.json\"", record.Snapshot.ID))
	w.Header().Set("X-Content-SHA256", record.DocumentSHA256)
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write(raw)
}
