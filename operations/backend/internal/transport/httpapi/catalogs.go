package httpapi

import (
	"net/http"

	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/strictjson"
)

type approveCatalogFields commercial.ApproveCatalogInput
type approveCatalogRequest struct {
	approveCatalogFields
	CurrentPassword string `json:"current_password"`
}

type exportCatalogRequest struct {
	CurrentPassword string `json:"current_password"`
}

func (v *exportCatalogRequest) UnmarshalJSON(data []byte) error {
	type wire exportCatalogRequest
	var result wire
	if err := strictjson.Object(data, &result, "current_password"); err != nil {
		return err
	}
	*v = exportCatalogRequest(result)
	return nil
}

func (v *approveCatalogRequest) UnmarshalJSON(data []byte) error {
	type wire approveCatalogRequest
	var result wire
	if err := strictjson.Object(data, &result, "request", "expected_public_sha256", "current_password"); err != nil {
		return err
	}
	*v = approveCatalogRequest(result)
	return nil
}
func (api *API) previewPublicCatalogHandler(w http.ResponseWriter, r *http.Request) {
	actor, _, ok := api.authorizeMutation(w, r)
	if !ok {
		return
	}
	var in commercial.CatalogRequest
	if err := api.decodeJSON(w, r, &in); err != nil {
		writeError(w, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	preview, err := api.service.PreviewPublicCatalog(r.Context(), in, actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_CATALOG_FAILED", "预览公开套餐失败", err)
		return
	}
	writeJSON(w, http.StatusOK, preview)
}
func (api *API) approvePublicCatalogHandler(w http.ResponseWriter, r *http.Request) {
	actor, _, ok := api.authorizeMutation(w, r)
	if !ok {
		return
	}
	var in approveCatalogRequest
	if err := api.decodeJSON(w, r, &in); err != nil {
		writeError(w, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	if err := api.service.Reauthenticate(r.Context(), actor, in.CurrentPassword); err != nil {
		writeError(w, http.StatusUnauthorized, "REAUTHENTICATION_FAILED", "当前密码验证失败")
		return
	}
	in.CurrentPassword = ""
	record, err := api.service.ApprovePublicCatalog(r.Context(), commercial.ApproveCatalogInput(in.approveCatalogFields), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_CATALOG_FAILED", "批准公开套餐失败", err)
		return
	}
	writeJSON(w, http.StatusCreated, record)
}
func (api *API) listCatalogApprovalsHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	records, err := api.service.ListCatalogApprovals(r.Context(), queryLimit(r, 20, 100), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_CATALOG_FAILED", "读取公开目录失败", err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"items": records})
}
func (api *API) getCatalogApprovalHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	record, err := api.service.GetCatalogApproval(r.Context(), r.PathValue("catalogID"), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_CATALOG_FAILED", "读取公开目录失败", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}
func (api *API) exportPublicCatalogHandler(w http.ResponseWriter, r *http.Request) {
	actor, _, ok := api.authorizeMutation(w, r)
	if !ok {
		return
	}
	// No caller-controlled path, environment, rights or bytes are accepted.
	var input exportCatalogRequest
	if err := api.decodeJSON(w, r, &input); err != nil {
		writeError(w, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	if err := api.service.Reauthenticate(r.Context(), actor, input.CurrentPassword); err != nil {
		writeError(w, http.StatusUnauthorized, "REAUTHENTICATION_FAILED", "当前密码验证失败")
		return
	}
	input.CurrentPassword = ""
	record, err := api.service.ExportPublicCatalog(r.Context(), r.PathValue("catalogID"), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_CATALOG_FAILED", "导出公开目录失败，请核对配置和原记录后重试", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}
func (api *API) downloadPublicCatalogHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	record, raw, err := api.service.DownloadPublicCatalog(r.Context(), r.PathValue("catalogID"), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_CATALOG_FAILED", "读取公开文件失败，请重新核对导出", err)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("Content-Disposition", `attachment; filename="plans.json"`)
	w.Header().Set("X-Content-SHA256", record.Public.SHA256)
	w.Header().Set("X-Catalog-Revision", record.Snapshot.ID)
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write(raw)
}
