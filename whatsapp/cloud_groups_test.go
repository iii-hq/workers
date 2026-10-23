package main

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"image"
	"image/jpeg"
	"io"
	"net/http"
	"strings"
	"testing"
)

const cloudGroupTestID = "Y2FwaV9ncm91cDoxNzA1NTU1MDEzOToxMjAzNjM0MDQ2OTQyMzM4MjAZD"

// cloudGroupOwned answers the ownership lookup that precedes group mutations.
func cloudGroupOwned(w http.ResponseWriter, r *http.Request) bool {
	if r.Method != http.MethodGet {
		return false
	}
	_, _ = io.WriteString(w, `{"data":{"groups":[{"id":"`+cloudGroupTestID+`"}]}}`)
	return true
}

func TestCloudGroupsScopedManagement(t *testing.T) {
	for _, tt := range []struct {
		operation, method, path string
		input                   map[string]any
		bodyKey                 string
		bodyValue               any
	}{
		{"create", "POST", "/v26.0/456/groups", map[string]any{"subject": "Support", "description": "Project", "join_approval_mode": "approval_required"}, "subject", "Support"},
		{"update", "POST", "/v26.0/" + cloudGroupTestID, map[string]any{"group_id": cloudGroupTestID, "subject": "Updated"}, "subject", "Updated"},
		{"invite_link::reset", "POST", "/v26.0/" + cloudGroupTestID + "/invite_link", map[string]any{"group_id": cloudGroupTestID}, "messaging_product", "whatsapp"},
		{"join_requests::approve", "POST", "/v26.0/" + cloudGroupTestID + "/join_requests", map[string]any{"group_id": cloudGroupTestID, "join_request_ids": []any{"join_request_1"}}, "messaging_product", "whatsapp"},
		{"join_requests::reject", "DELETE", "/v26.0/" + cloudGroupTestID + "/join_requests", map[string]any{"group_id": cloudGroupTestID, "join_request_ids": []any{"join_request_1"}}, "messaging_product", "whatsapp"},
		{"participants::remove", "DELETE", "/v26.0/" + cloudGroupTestID + "/participants", map[string]any{"group_id": cloudGroupTestID, "participant_ids": []any{"+15551234567"}}, "messaging_product", "whatsapp"},
		{"delete", "DELETE", "/v26.0/" + cloudGroupTestID, map[string]any{"group_id": cloudGroupTestID}, "", nil},
	} {
		t.Run(tt.operation, func(t *testing.T) {
			writes := 0
			checks := 0
			client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
				if r.Method == "GET" && r.URL.Path == "/v26.0/456/groups" {
					checks++
					_, _ = io.WriteString(w, `{"data":{"groups":[{"id":"`+cloudGroupTestID+`"}]}}`)
					return
				}
				writes++
				if r.Method != tt.method || r.URL.Path != tt.path {
					t.Errorf("unexpected route %s %s", r.Method, r.URL.Path)
				}
				if tt.bodyKey != "" {
					var payload map[string]any
					if json.NewDecoder(r.Body).Decode(&payload) != nil || payload[tt.bodyKey] != tt.bodyValue {
						t.Errorf("incorrect body: %#v", payload)
					}
				}
				response := `{"success":true}`
				switch tt.operation {
				case "invite_link::reset":
					response = `{"messaging_product":"whatsapp","invite_link":"https://chat.whatsapp.com/VerifiedInvite"}`
				case "join_requests::approve":
					response = `{"messaging_product":"whatsapp","approved_join_requests":["join_request_1"]}`
				case "join_requests::reject":
					response = `{"messaging_product":"whatsapp","rejected_join_requests":["join_request_1"]}`
				}
				_, _ = io.WriteString(w, response)
			})
			_, err := client.manageGroup(context.Background(), cloudTestConfig(), tt.operation, tt.input)
			if err != nil {
				t.Fatal(err)
			}
			if writes != 1 || tt.operation != "create" && checks != 1 {
				t.Fatalf("writes=%d scope checks=%d", writes, checks)
			}
		})
	}
}

func TestCloudGroupsOwnershipAndValidation(t *testing.T) {
	for _, tt := range []struct {
		name, operation string
		input           map[string]any
		code            string
		wantCalls       int
	}{
		{"unowned", "delete", map[string]any{"group_id": cloudGroupTestID}, "cloud_group_not_found", 1},
		{"path", "get", map[string]any{"group_id": "../456"}, "invalid_request", 0},
		{"empty subject", "create", map[string]any{"subject": " "}, "invalid_request", 0},
		{"native participants", "participants::remove", map[string]any{"group_id": cloudGroupTestID, "participant_ids": []any{"123@g.us"}}, "invalid_request", 0},
		{"unknown field", "get", map[string]any{"group_id": cloudGroupTestID, "access_token": "override"}, "invalid_request", 0},
	} {
		t.Run(tt.name, func(t *testing.T) {
			calls := 0
			client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
				calls++
				_, _ = io.WriteString(w, `{"data":{"groups":[]}}`)
			})
			_, err := client.manageGroup(context.Background(), cloudTestConfig(), tt.operation, tt.input)
			assertCloudDocumentError(t, err, tt.code)
			if calls != tt.wantCalls {
				t.Fatalf("calls=%d", calls)
			}
		})
	}
}

func TestCloudGroupsSendPayloads(t *testing.T) {
	for _, tt := range []struct {
		operation string
		input     map[string]any
		kind      string
	}{
		{"send_text", map[string]any{"group_id": cloudGroupTestID, "text": "Hello"}, "text"},
		{"send_media", map[string]any{"group_id": cloudGroupTestID, "media_type": "image", "media_id": "987", "caption": "Image"}, "image"},
		{"send_template", map[string]any{"group_id": cloudGroupTestID, "name": "group_update", "language": "en_US"}, "template"},
		{"pin", map[string]any{"group_id": cloudGroupTestID, "message_id": "wamid.original", "expiration_days": 7}, "pin"},
		{"unpin", map[string]any{"group_id": cloudGroupTestID, "message_id": "wamid.original"}, "pin"},
	} {
		t.Run(tt.operation, func(t *testing.T) {
			id, body, kind, _, err := cloudGroupMessagePayload(tt.operation, tt.input)
			if err != nil {
				t.Fatal(err)
			}
			if id != cloudGroupTestID || kind != tt.kind || body["type"] != tt.kind || body["to"] != cloudGroupTestID || body["recipient_type"] != "group" {
				t.Fatalf("incorrect group message: %#v %s %s", body, id, kind)
			}
			if body["recipient"] != nil {
				t.Error("individual recipient leaked into group message")
			}
		})
	}
	_, _, _, _, err := cloudGroupMessagePayload("send_media", map[string]any{"group_id": cloudGroupTestID, "media_type": "interactive", "media_id": "987"})
	assertCloudDocumentError(t, err, "invalid_request")
}

func TestCloudGroupMissingAcknowledgementIsAmbiguousWithoutReplay(t *testing.T) {
	for _, operation := range []string{"create", "update", "delete", "invite_link::reset", "join_requests::approve", "join_requests::reject", "participants::remove"} {
		t.Run(operation, func(t *testing.T) {
			writes := 0
			client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
				if cloudGroupOwned(w, r) {
					return
				}
				writes++
				_, _ = io.WriteString(w, `{}`)
			})
			input := map[string]any{"group_id": cloudGroupTestID}
			switch operation {
			case "create":
				input = map[string]any{"subject": "Example"}
			case "update":
				input["subject"] = "Example"
			case "join_requests::approve", "join_requests::reject":
				input["join_request_ids"] = []any{"join_request_1"}
			case "participants::remove":
				input["participant_ids"] = []any{"+15551234567"}
			}
			_, err := client.manageGroup(context.Background(), cloudTestConfig(), operation, input)
			assertCloudDocumentError(t, err, "cloud_outcome_unknown")
			if writes != 1 {
				t.Fatal("group mutation was replayed")
			}
		})
	}
}

func TestCloudGroupJoinAcknowledgementPreservesPartialFailures(t *testing.T) {
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		if cloudGroupOwned(w, r) {
			return
		}
		_, _ = io.WriteString(w, `{"approved_join_requests":["join_request_1"],"failed_join_requests":[{"join_request_id":"join_request_2","errors":[{"code":131000,"message":"test-access-secret"}]}]}`)
	})
	result, err := client.manageGroup(context.Background(), cloudTestConfig(), "join_requests::approve", map[string]any{"group_id": cloudGroupTestID, "join_request_ids": []any{"join_request_1", "join_request_2"}})
	if err != nil {
		t.Fatal(err)
	}
	if len(result["approved_join_requests"].([]any)) != 1 || len(result["failed_join_requests"].([]any)) != 1 {
		t.Fatal("partial membership outcome lost")
	}
	failed := result["failed_join_requests"].([]any)[0].(map[string]any)
	if failed["errors"].([]any)[0].(map[string]any)["message"] == "test-access-secret" {
		t.Fatal("partial failure exposed a credential")
	}
}

func TestCloudGroupsPhotoMultipart(t *testing.T) {
	var picture bytes.Buffer
	if err := jpeg.Encode(&picture, image.NewRGBA(image.Rect(0, 0, 192, 192)), nil); err != nil {
		t.Fatal(err)
	}
	writes := 0
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		if cloudGroupOwned(w, r) {
			return
		}
		writes++
		if r.ContentLength < 1 {
			t.Error("photo upload must carry Content-Length")
		}
		reader, err := r.MultipartReader()
		if err != nil {
			t.Error(err)
			return
		}
		found := false
		for {
			part, err := reader.NextPart()
			if err == io.EOF {
				break
			}
			if err != nil {
				t.Error(err)
				return
			}
			data, _ := io.ReadAll(part)
			if part.FormName() == "file" {
				found = true
				if !bytes.Equal(data, picture.Bytes()) || part.Header.Get("Content-Type") != "image/jpeg" || part.FileName() != "group.jpg" {
					t.Error("wrong photo multipart")
				}
			}
		}
		if !found {
			t.Error("missing multipart file")
		}
		_, _ = io.WriteString(w, `{"success":true}`)
	})
	_, err := client.manageGroup(context.Background(), cloudTestConfig(), "photo::set", map[string]any{"group_id": cloudGroupTestID, "data_base64": base64.StdEncoding.EncodeToString(picture.Bytes()), "file_name": "group.jpg", "mimetype": "image/jpeg"})
	if err != nil || writes != 1 {
		t.Fatalf("err=%v writes=%d", err, writes)
	}
}

func TestCloudGroupsArchiveAndNoRetry(t *testing.T) {
	config := cloudTestConfig()
	sends := 0
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		if cloudGroupOwned(w, r) {
			return
		}
		sends++
		_, _ = io.WriteString(w, `{"messages":[{"id":"wamid.group"}]}`)
	})
	registry := cloudTestRegistry(t, client, config, registerCloudGroups)
	result, err := registry.Invoke(context.Background(), "whatsapp::cloud::groups::messages::send_text", map[string]any{"group_id": cloudGroupTestID, "text": "Group message"})
	if err != nil {
		t.Fatal(err)
	}
	sent, ok := result.(CloudSentMessage)
	if !ok || sent.ChatID != "group:"+cloudGroupTestID || sent.Status != "accepted" || sends != 1 {
		t.Fatalf("unexpected group send: %+v", result)
	}
	inbox, err := registry.Manager.CloudInbox(config)
	if err != nil {
		t.Fatal(err)
	}
	page, err := inbox.ListMessages("cloud:group:"+cloudGroupTestID, "", 10)
	if err != nil || len(page.Messages) != 1 || page.Messages[0].Text != "Group message" {
		t.Fatalf("archive: %+v %v", page, err)
	}
	calls := 0
	broken := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		calls++
		if !cloudGroupOwned(w, r) {
			_, _ = io.WriteString(w, "test-access-secret")
		}
	})
	_, err = broken.manageGroup(context.Background(), config, "delete", map[string]any{"group_id": cloudGroupTestID})
	assertCloudDocumentError(t, err, "cloud_outcome_unknown")
	if calls != 2 {
		t.Errorf("group deletion retried: %d", calls)
	}
}

func TestCloudGroupsOwnershipPaginationAndSafeResult(t *testing.T) {
	calls := 0
	client := cloudTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		calls++
		if r.URL.Path == "/v26.0/456/groups" {
			if r.URL.Query().Get("after") == "" {
				_, _ = io.WriteString(w, `{"data":{"groups":[{"id":"other"}]},"paging":{"cursors":{"after":"next-page"}}}`)
			} else {
				_, _ = io.WriteString(w, `{"data":{"groups":[{"id":"`+cloudGroupTestID+`"}]}}`)
			}
			return
		}
		_, _ = io.WriteString(w, `{"id":"`+cloudGroupTestID+`","access_token":"test-access-secret","description":"test-app-secret"}`)
	})
	result, err := client.manageGroup(context.Background(), cloudTestConfig(), "get", map[string]any{"group_id": cloudGroupTestID})
	if err != nil {
		t.Fatal(err)
	}
	raw, _ := json.Marshal(result)
	if strings.Contains(string(raw), "test-access-secret") || strings.Contains(string(raw), "test-app-secret") || calls != 3 {
		t.Fatalf("unsafe result or wrong pagination: %s calls=%d", raw, calls)
	}
}
