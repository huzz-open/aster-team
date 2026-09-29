package httpapi

import (
	"net/http"

	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/strictjson"
)

type confirmationPaymentFields commercial.ConfirmPaymentInput
type confirmCommercialPaymentInput struct {
	confirmationPaymentFields
	CurrentPassword string `json:"current_password"`
}

func (v *confirmCommercialPaymentInput) UnmarshalJSON(data []byte) error {
	type wire confirmCommercialPaymentInput
	var result wire
	if err := strictjson.Object(data, &result, "operation_id", "expected_order_sha256", "payment_reference", "received_at", "notes", "current_password"); err != nil {
		return err
	}
	*v = confirmCommercialPaymentInput(result)
	return nil
}
func (api *API) confirmCommercialPaymentHandler(w http.ResponseWriter, r *http.Request) {
	actor, _, ok := api.authorizeMutation(w, r)
	if !ok {
		return
	}
	var in confirmCommercialPaymentInput
	if err := api.decodeJSON(w, r, &in); err != nil {
		writeError(w, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	if err := api.service.Reauthenticate(r.Context(), actor, in.CurrentPassword); err != nil {
		writeError(w, http.StatusUnauthorized, "REAUTHENTICATION_FAILED", "当前密码验证失败")
		return
	}
	in.CurrentPassword = ""
	record, err := api.service.ConfirmCommercialPayment(r.Context(), r.PathValue("orderID"), commercial.ConfirmPaymentInput(in.confirmationPaymentFields), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "PAYMENT_CONFIRM_FAILED", "确认订单到账失败", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}
func (api *API) getCommercialPaymentHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	record, err := api.service.GetCommercialPayment(r.Context(), r.PathValue("orderID"), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "ORDERS_LIST_FAILED", "读取到账记录失败", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}
func (api *API) commercialPaymentContextHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	result, err := api.service.GetCommercialPaymentContext(r.Context(), r.PathValue("orderID"), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "ORDERS_LIST_FAILED", "读取待核对订单失败", err)
		return
	}
	writeJSON(w, http.StatusOK, result)
}
