package httpapi

import (
	"encoding/json"
	"fmt"
	"net/http"

	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/strictjson"
)

// The method-free embedded payload keeps authentication material outside the
// commercial input passed to the service, store and audit trail.
type approvalDistributionFields commercial.ApproveDistributionInput

type approveDistributionInput struct {
	approvalDistributionFields
	CurrentPassword string `json:"current_password"`
}

func (v *approveDistributionInput) UnmarshalJSON(data []byte) error {
	type wire approveDistributionInput
	var result wire
	if err := strictjson.Object(data, &result, "operation_id", "plan_id", "plan_version", "expected_sha256", "not_before", "reason", "current_password"); err != nil {
		return err
	}
	*v = approveDistributionInput(result)
	return nil
}

type issueDistributionInput struct {
	KeyID           string `json:"key_id"`
	CurrentPassword string `json:"current_password"`
}

func (v *issueDistributionInput) UnmarshalJSON(data []byte) error {
	type wire issueDistributionInput
	var result wire
	if err := strictjson.Object(data, &result, "key_id", "current_password"); err != nil {
		return err
	}
	*v = issueDistributionInput(result)
	return nil
}

func (api *API) freeDistributionsHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	records, err := api.service.ListFreeDistributions(r.Context(), queryLimit(r, 50, 100), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_DISTRIBUTION_FAILED", "读取免费分发记录失败", err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"items": records})
}
func (api *API) approveFreeDistributionHandler(w http.ResponseWriter, r *http.Request) {
	actor, _, ok := api.authorizeMutation(w, r)
	if !ok {
		return
	}
	var in approveDistributionInput
	if err := api.decodeJSON(w, r, &in); err != nil {
		writeError(w, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	if err := api.service.Reauthenticate(r.Context(), actor, in.CurrentPassword); err != nil {
		writeError(w, http.StatusUnauthorized, "REAUTHENTICATION_FAILED", "当前密码验证失败")
		return
	}
	in.CurrentPassword = ""
	record, err := api.service.ApproveFreeDistribution(r.Context(), commercial.ApproveDistributionInput(in.approvalDistributionFields), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_DISTRIBUTION_FAILED", "批准免费分发失败", err)
		return
	}
	writeJSON(w, http.StatusCreated, record)
}
func (api *API) getFreeDistributionHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	record, err := api.service.GetFreeDistribution(r.Context(), r.PathValue("distributionID"), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_DISTRIBUTION_FAILED", "读取免费分发记录失败", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}
func (api *API) issueFreeDistributionHandler(w http.ResponseWriter, r *http.Request) {
	actor, _, ok := api.authorizeMutation(w, r)
	if !ok {
		return
	}
	var in issueDistributionInput
	if err := api.decodeJSON(w, r, &in); err != nil {
		writeError(w, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	if err := api.service.Reauthenticate(r.Context(), actor, in.CurrentPassword); err != nil {
		writeError(w, http.StatusUnauthorized, "REAUTHENTICATION_FAILED", "当前密码验证失败")
		return
	}
	in.CurrentPassword = ""
	record, err := api.service.IssueFreeDistribution(r.Context(), r.PathValue("distributionID"), in.KeyID, actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_DISTRIBUTION_FAILED", "签发免费授权失败", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}
func (api *API) v2IssuerProfilesHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	profiles, err := api.service.ListV2IssuerProfiles(r.Context(), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_DISTRIBUTION_FAILED", "读取签发配置失败", err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"items": profiles})
}

func (api *API) commercialEnvironmentHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	environment, err := api.service.GetFulfillmentEnvironment(r.Context(), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_DISTRIBUTION_FAILED", "读取履约环境失败", err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]string{"fulfillment_environment": environment})
}
func (api *API) downloadFreeDistributionHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	record, err := api.service.GetFreeDistribution(r.Context(), r.PathValue("distributionID"), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_DISTRIBUTION_FAILED", "读取授权文件失败", err)
		return
	}
	if record.Document == nil {
		writeError(w, http.StatusConflict, "COMMERCIAL_DISTRIBUTION_NOT_ISSUED", "该分发记录尚未完成签发")
		return
	}
	// Marshal identically to persistence so the advertised digest covers the
	// exact download, without newline/indentation transformations.
	raw, err := json.Marshal(record.Document)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_DISTRIBUTION_FAILED", "生成授权文件失败", err)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("Content-Disposition", fmt.Sprintf("attachment; filename=\"aster-free-%s.license.json\"", record.Snapshot.ID))
	w.Header().Set("X-Content-SHA256", record.DocumentSHA256)
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write(raw)
}
