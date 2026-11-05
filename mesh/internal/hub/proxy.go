package hub

import "maps"

func EnsureProxyConfig(current map[string]any) (next map[string]any, write bool, status string) {
	if value, exists := current["rbac"]; exists && value != nil {
		rbac, ok := value.(map[string]any)
		if !ok {
			return current, false, "invalid rbac configuration"
		}
		if value, exists := rbac["auth_function_id"]; exists && value != nil {
			auth, ok := value.(string)
			if !ok {
				return current, false, "invalid auth function"
			}
			if auth == "mesh::auth" {
				// Only upgrade the exact removed Mesh-owned hook. Conflicts retain
				// their authoritative bytes and are refused by validProxyHooks.
				if current["middleware_function_id"] == "mesh::middleware" && rbac["on_trigger_registration_function_id"] == "mesh::on-trigger-registration" {
					next = maps.Clone(current)
					nextRBAC := maps.Clone(rbac)
					next["rbac"] = nextRBAC
					delete(nextRBAC, "on_trigger_registration_function_id")
					if hook := nextRBAC["on_observer_registration_function_id"]; hook == nil || hook == "" || hook == "mesh::on-observer-registration" {
						nextRBAC["on_observer_registration_function_id"] = "mesh::on-observer-registration"
						if validProxyHooks(next) {
							return next, true, "ok"
						}
					}
				}
				return current, false, "ok"
			}
			if auth != "" {
				return current, false, "owned by " + auth
			}
		}
	}
	return map[string]any{"host": "127.0.0.1", "port": 49200, "engine_url": "ws://127.0.0.1:49134", "middleware_function_id": "mesh::middleware", "rbac": map[string]any{"auth_function_id": "mesh::auth", "on_observer_registration_function_id": "mesh::on-observer-registration"}}, true, "ok"
}
