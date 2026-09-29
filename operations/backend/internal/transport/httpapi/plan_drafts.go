package httpapi

import (
	"net/http"
	"strconv"

	"aster.local/team/operations/backend/internal/application"
)

func (api *API) listPlanDraftsHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	items, err := api.service.ListPlanDrafts(r.Context(), queryLimit(r, 50, 100), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_DRAFT_FAILED", "读取套餐草稿失败", err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"items": items})
}

func (api *API) getPlanDraftHandler(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authenticate(w, r)
	if !ok {
		return
	}
	var revision uint64
	var err error
	if value := r.URL.Query().Get("revision"); value != "" {
		revision, err = strconv.ParseUint(value, 10, 32)
		if err != nil || revision == 0 {
			writeError(w, http.StatusBadRequest, "VALIDATION_FAILED", "草稿修订号无效")
			return
		}
	}
	record, err := api.service.GetPlanDraft(r.Context(), r.PathValue("draftID"), uint32(revision), actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_DRAFT_FAILED", "读取套餐草稿失败", err)
		return
	}
	writeJSON(w, http.StatusOK, record)
}

func (api *API) savePlanDraftHandler(w http.ResponseWriter, r *http.Request) {
	actor, _, ok := api.authorizeMutation(w, r)
	if !ok {
		return
	}
	var in application.SavePlanDraftInput
	if err := api.decodeJSON(w, r, &in); err != nil {
		writeError(w, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	record, err := api.service.SavePlanDraft(r.Context(), in, actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_DRAFT_FAILED", "保存套餐草稿失败", err)
		return
	}
	writeJSON(w, http.StatusCreated, record)
}

func (api *API) freezePlanDraftHandler(w http.ResponseWriter, r *http.Request) {
	actor, _, ok := api.authorizeMutation(w, r)
	if !ok {
		return
	}
	var in application.FreezePlanDraftInput
	if err := api.decodeJSON(w, r, &in); err != nil {
		writeError(w, http.StatusBadRequest, "INVALID_JSON", err.Error())
		return
	}
	record, err := api.service.FreezePlanDraft(r.Context(), in, actor.ID)
	if err != nil {
		api.writeCommercialError(w, "COMMERCIAL_DRAFT_FAILED", "生成套餐版本失败", err)
		return
	}
	writeJSON(w, http.StatusCreated, record)
}
