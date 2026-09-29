package httpapi

import (
	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/domain"
	"errors"
	"net/http"
	"strconv"
)

func (api *API) rotateUpgradeCredentials(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authorizeMutationPermission(w, r, application.PermissionEnvironmentWrite)
	if !ok {
		return
	}
	var input domain.UpgradeCredentials
	if api.decodeJSON(w, r, &input) != nil {
		writeError(w, http.StatusBadRequest, "INVALID_JSON", "凭据格式无效")
		return
	}
	if err := api.service.RotateUpgradeCredentials(r.Context(), r.PathValue("environmentID"), input, actor.ID); err != nil {
		api.environmentUpgradeError(w, err)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

func (api *API) environmentUpgradeError(w http.ResponseWriter, err error) {
	status := http.StatusServiceUnavailable
	message := "环境升级暂时不可用，请检查目标连接、制品和凭据加密配置"
	if errors.Is(err, application.ErrValidation) {
		status = http.StatusBadRequest
		message = err.Error()
	}
	if errors.Is(err, application.ErrNotFound) {
		status = http.StatusNotFound
		message = "目标环境或升级任务不存在"
	}
	if errors.Is(err, application.ErrUnauthorized) {
		status = http.StatusForbidden
		message = "当前账号没有环境升级权限"
	}
	writeError(w, status, "ENVIRONMENT_UPGRADE_FAILED", message)
}

func (api *API) listUpgradeEnvironments(w http.ResponseWriter, r *http.Request) {
	if _, ok := api.authorizePermission(w, r, application.PermissionReleaseRead); !ok {
		return
	}
	items, err := api.service.ListUpgradeEnvironments(r.Context())
	if err != nil {
		api.environmentUpgradeError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"items": items})
}

func (api *API) createUpgradeEnvironment(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authorizeMutationPermission(w, r, application.PermissionEnvironmentWrite)
	if !ok {
		return
	}
	var input domain.UpgradeEnvironmentInput
	if api.decodeJSON(w, r, &input) != nil {
		writeError(w, http.StatusBadRequest, "INVALID_JSON", "环境配置格式无效")
		return
	}
	item, err := api.service.CreateUpgradeEnvironment(r.Context(), input, actor.ID)
	if err != nil {
		api.environmentUpgradeError(w, err)
		return
	}
	writeJSON(w, http.StatusCreated, item)
}

func (api *API) inspectUpgradeEnvironment(w http.ResponseWriter, r *http.Request) {
	// Inspection signs in to the target, so require mutation authorization too.
	if _, ok := api.authorizeMutationPermission(w, r, application.PermissionEnvironmentUpgrade); !ok {
		return
	}
	item, err := api.service.InspectUpgradeEnvironment(r.Context(), r.PathValue("environmentID"))
	if err != nil {
		api.environmentUpgradeError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, item)
}

func (api *API) createEnvironmentUpgrade(w http.ResponseWriter, r *http.Request) {
	actor, ok := api.authorizeMutationPermission(w, r, application.PermissionEnvironmentUpgrade)
	if !ok {
		return
	}
	var input struct {
		EnvironmentID string `json:"environment_id"`
		ArtifactID    string `json:"artifact_id"`
	}
	if api.decodeJSON(w, r, &input) != nil {
		writeError(w, http.StatusBadRequest, "INVALID_JSON", "升级参数无效")
		return
	}
	item, err := api.service.CreateEnvironmentUpgrade(r.Context(), input.EnvironmentID, input.ArtifactID, actor.ID)
	if err != nil {
		api.environmentUpgradeError(w, err)
		return
	}
	writeJSON(w, http.StatusAccepted, item)
}

func (api *API) listEnvironmentUpgrades(w http.ResponseWriter, r *http.Request) {
	if _, ok := api.authorizePermission(w, r, application.PermissionReleaseRead); !ok {
		return
	}
	items, err := api.service.ListEnvironmentUpgrades(r.Context())
	if err != nil {
		api.environmentUpgradeError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"items": items})
}

func (api *API) getEnvironmentUpgrade(w http.ResponseWriter, r *http.Request) {
	if _, ok := api.authorizePermission(w, r, application.PermissionReleaseRead); !ok {
		return
	}
	after := int64(0)
	if value := r.URL.Query().Get("after"); value != "" {
		var err error
		after, err = strconv.ParseInt(value, 10, 64)
		if err != nil || after < 0 {
			writeError(w, http.StatusBadRequest, "INVALID_JSON", "采样游标无效")
			return
		}
	}
	item, err := api.service.GetEnvironmentUpgrade(r.Context(), r.PathValue("upgradeID"), after)
	if err != nil {
		api.environmentUpgradeError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, item)
}
