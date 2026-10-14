package main

import (
	"errors"
	"os"
)

var cloudEnvironment = map[string]string{
	"enabled":                "WHATSAPP_CLOUD_ENABLED",
	"api_version":            "WHATSAPP_CLOUD_API_VERSION",
	"business_account_id":    "WHATSAPP_CLOUD_BUSINESS_ACCOUNT_ID",
	"phone_number_id":        "WHATSAPP_CLOUD_PHONE_NUMBER_ID",
	"access_token":           "WHATSAPP_CLOUD_ACCESS_TOKEN",
	"app_secret":             "WHATSAPP_META_APP_SECRET",
	"verify_token":           "WHATSAPP_WEBHOOK_VERIFY_TOKEN",
	"flow_private_key":       "WHATSAPP_FLOW_PRIVATE_KEY",
	"flow_handler_function":  "WHATSAPP_FLOW_HANDLER_FUNCTION",
	"flow_handler_namespace": "WHATSAPP_FLOW_HANDLER_NAMESPACE",
}

// Process overrides are runtime-only: keep the original Config for seeding the
// central store. LookupEnv distinguishes an absent variable from an empty one.
func runtimeConfig(c Config) (Config, error) {
	if value, set := os.LookupEnv(cloudEnvironment["enabled"]); set {
		switch value {
		case "true":
			c.Cloud.Enabled = true
		case "false":
			c.Cloud.Enabled = false
		default:
			return c, errors.New("WHATSAPP_CLOUD_ENABLED must be true or false")
		}
	}
	for _, field := range []struct {
		name   string
		target *string
	}{
		{"api_version", &c.Cloud.APIVersion},
		{"business_account_id", &c.Cloud.BusinessAccountID},
		{"phone_number_id", &c.Cloud.PhoneNumberID},
		{"access_token", &c.Cloud.AccessToken},
		{"app_secret", &c.Cloud.AppSecret},
		{"verify_token", &c.Cloud.VerifyToken},
		{"flow_private_key", &c.Cloud.FlowPrivateKey},
		{"flow_handler_function", &c.Cloud.FlowHandlerFunction},
		{"flow_handler_namespace", &c.Cloud.FlowHandlerNamespace},
	} {
		if value, set := os.LookupEnv(cloudEnvironment[field.name]); set {
			*field.target = value
		}
	}
	return c, c.Validate()
}

func cloudEnvironmentOverrides() map[string]string {
	fields := make(map[string]string)
	for field, variable := range cloudEnvironment {
		if _, set := os.LookupEnv(variable); set {
			fields["cloud."+field] = variable
		}
	}
	return fields
}
