package main

import (
	"context"
	"encoding/json"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

func cloudVerificationHandler(manager *Manager) iii.TypedHandler[callbackRequest[cloudVerifyRequest], cloudHTTPResponse] {
	return func(parent context.Context, input callbackRequest[cloudVerifyRequest]) (cloudHTTPResponse, error) {
		if manager.interfaceOnly {
			return cloudHTTPText(503, "Interface capture does not execute callbacks"), nil
		}
		var req cloudVerifyRequest
		if err := json.Unmarshal(input.Raw, &req); err != nil {
			return cloudHTTPText(400, "Invalid verification request"), nil
		}
		ctx, cancel := context.WithTimeout(parent, 8*time.Second)
		defer cancel()
		config, err := manager.ConfigContext(ctx)
		if err != nil {
			return cloudHTTPText(503, "Webhook verification is unavailable; retry later"), nil
		}
		return verifyCloudWebhook(config.Cloud, req.QueryParams), nil
	}
}

func cloudFlowEndpointHandler(client *iii.Client, manager *Manager, engineURL string) iii.TypedHandler[callbackRequest[cloudReceiveRequest], cloudHTTPResponse] {
	return func(parent context.Context, input callbackRequest[cloudReceiveRequest]) (cloudHTTPResponse, error) {
		if manager.interfaceOnly {
			return cloudHTTPText(503, "Interface capture does not execute callbacks"), nil
		}
		var req cloudReceiveRequest
		if json.Unmarshal(input.Raw, &req) != nil {
			return cloudHTTPText(400, "Invalid request"), nil
		}
		ctx, cancel := context.WithTimeout(parent, 10*time.Second)
		defer cancel()
		config, err := manager.ConfigContext(ctx)
		if err != nil {
			return cloudHTTPText(503, "Flow endpoint is unavailable; retry later"), nil
		}
		if !config.Cloud.Enabled || config.Cloud.FlowPrivateKey == "" || config.Cloud.AppSecret == "" {
			return cloudHTTPText(503, "Flow endpoint is not configured"), nil
		}
		if _, err := cloudSignatureBytes(cloudHeader(req.Headers, "x-hub-signature-256")); err != nil {
			return cloudHTTPText(403, "Invalid signature"), nil
		}
		raw, status := readCloudChannel(ctx, engineURL, req.RequestBody, cloudFlowEnvelopeLimit)
		if status != 0 {
			return cloudHTTPText(status, map[int]string{400: "Original request body is required", 413: "Flow payload exceeds the size limit", 503: "Unable to read Flow request"}[status]), nil
		}
		handler := func(callCtx context.Context, data json.RawMessage) (json.RawMessage, error) {
			namespace := config.Cloud.FlowHandlerNamespace
			if namespace == "" {
				namespace = Namespace()
			}
			return client.Trigger(callCtx, iii.TriggerRequest{FunctionID: config.Cloud.FlowHandlerFunction, Namespace: namespace, Data: data, Timeout: 8 * time.Second})
		}
		return processCloudFlow(ctx, config, raw, cloudHeader(req.Headers, "x-hub-signature-256"), handler), nil
	}
}
