package httpapi

import (
	"net/http"

	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/strictjson"
)

type preparePublicationFields commercial.PreparePublicationInput
type preparePublicationRequest struct {
	preparePublicationFields
	CurrentPassword string `json:"current_password"`
}

func (v *preparePublicationRequest) UnmarshalJSON(data []byte) error {
	type wire preparePublicationRequest
	return strictjson.Object(data, (*wire)(v), "operation_id", "catalog_revision", "build_sha256", "expected_active_id", "accept_until", "reason", "current_password")
}
func (api *API) preparePublicationHandler(w http.ResponseWriter, r *http.Request) {
	actor, _, ok := api.authorizeMutation(w, r)
	if !ok {
		return
	}
	var in preparePublicationRequest
	if err := api.decodeJSON(w, r, &in); err != nil {
		writeError(w, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	if err := api.service.Reauthenticate(r.Context(), actor, in.CurrentPassword); err != nil {
		writeError(w, http.StatusUnauthorized, "REAUTHENTICATION_FAILED", "当前密码验证失败")
		return
	}
	in.CurrentPassword = ""
	record, err := api.service.PreparePublication(r.Context(), commercial.PreparePublicationInput(in.preparePublicationFields), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_PUBLICATION_FAILED", "保存发布核对请求失败", err)
		return
	}
	writeJSON(w, http.StatusCreated, record)
}
func (api *API) getPublicationHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	record, err := api.service.GetPublication(r.Context(), r.PathValue("publicationID"), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_PUBLICATION_FAILED", "读取发布核对记录失败", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}
func (api *API) listPublicationsHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	records, err := api.service.ListPublications(r.Context(), queryLimit(r, 20, 100), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_PUBLICATION_FAILED", "读取发布核对记录失败", err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"items": records})
}
func (api *API) getPublicationHeadHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	id, err := api.service.GetPublicationHead(r.Context(), r.PathValue("environment"), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_PUBLICATION_FAILED", "读取环境发布记录失败", err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]string{"active_id": id})
}
func (api *API) acceptPublicationHandler(w http.ResponseWriter, r *http.Request) {
	actor, _, ok := api.authorizeMutation(w, r)
	if !ok {
		return
	}
	// Reuse the strict password-only transport. Target, evidence, entitlements
	// and acceptance deadline cannot be supplied or replaced in this request.
	var in exportCatalogRequest
	if err := api.decodeJSON(w, r, &in); err != nil {
		writeError(w, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	if err := api.service.Reauthenticate(r.Context(), actor, in.CurrentPassword); err != nil {
		writeError(w, http.StatusUnauthorized, "REAUTHENTICATION_FAILED", "当前密码验证失败")
		return
	}
	in.CurrentPassword = ""
	record, err := api.service.VerifyAndAcceptPublication(r.Context(), r.PathValue("publicationID"), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_PUBLICATION_FAILED", "核对发布并批准受理失败", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}

func (api *API) listPublicationFailuresHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	items, err := api.service.ListPublicationFailures(r.Context(), r.PathValue("publicationID"), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_PUBLICATION_FAILED", "读取核对失败记录失败", err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"items": items})
}
