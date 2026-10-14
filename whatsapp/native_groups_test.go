package main

import (
	"encoding/base64"
	"encoding/json"
	"errors"
	"reflect"
	"strings"
	"testing"
	"time"

	validate "github.com/santhosh-tekuri/jsonschema/v6"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/types"
)

func TestParseJIDPreservesBaselineSyntax(t *testing.T) {
	for input, want := range map[string]string{
		"+15551234567": "15551234567@s.whatsapp.net", "15551234567": "15551234567@s.whatsapp.net",
		"15551234567@c.us": "15551234567@s.whatsapp.net", "1234:65535@lid": "1234:65535@lid",
		"1234-5678@g.us": "1234-5678@g.us", "1234@newsletter": "1234@newsletter", "status@broadcast": "status@broadcast",
	} {
		jid, err := parseJID(input)
		if err != nil || jid.String() != want {
			t.Errorf("%q = %q, %v; want %q", input, jid.String(), err, want)
		}
	}
	for _, input := range []string{"", "123456", "+1234567890123456", " 15551234567", "15551234567 ", "abc@s.whatsapp.net", "12:65536@lid", "12:1:2@lid", "12.1:2@lid", "12:1@g.us", "12--34@g.us", "12-34-56@g.us", "status@newsletter", "1234@example.org", "1234@s.whatsapp.net@evil", "@lid", "12@", strings.Repeat("1", 129) + "@lid"} {
		if _, err := parseJID(input); err == nil {
			t.Errorf("invalid JID accepted: %q", input)
		}
	}
}

func TestGroupContactValidation(t *testing.T) {
	for _, input := range []string{"120363001@g.us", "12345-67890@g.us"} {
		if _, err := groupJID(input); err != nil {
			t.Errorf("group %q: %v", input, err)
		}
	}
	for _, input := range []string{"1234567@s.whatsapp.net", "12345@lid", "status@broadcast", "bad@g.us"} {
		if _, err := groupJID(input); err == nil {
			t.Errorf("accepted invalid group %q", input)
		}
	}
	for _, input := range []string{"120363001@g.us", "12345@newsletter", "status@broadcast"} {
		if _, err := contactJID(input); err == nil {
			t.Errorf("accepted invalid contact %q", input)
		}
	}
	for _, values := range [][]string{nil, {"1234567@s.whatsapp.net", "1234567:2@s.whatsapp.net"}, {"1234567@c.us", "1234567@s.whatsapp.net"}, make([]string, 1025)} {
		if _, err := groupContacts(values, false); err == nil {
			t.Errorf("accepted invalid contact list: %#v", values)
		}
	}
	if list, err := groupContacts(nil, true); err != nil || list == nil || len(list) != 0 {
		t.Fatalf("empty group creation: %#v, %v", list, err)
	}
	list, err := groupContacts([]string{"+15551234567", "99887766@lid"}, false)
	if err != nil || list[0].String() != "15551234567@s.whatsapp.net" || list[1].Server != types.HiddenUserServer {
		t.Fatalf("contact normalization: %#v, %v", list, err)
	}
}

func TestGroupTextAndJPEGValidation(t *testing.T) {
	if err := groupText(strings.Repeat("é", 100), "subject", 100, false); err != nil {
		t.Fatal(err)
	}
	for _, value := range []string{" ", strings.Repeat("é", 101)} {
		if err := groupText(value, "subject", 100, false); err == nil {
			t.Errorf("accepted invalid subject %q", value)
		}
	}
	if err := groupText("", "description", 2048, true); err != nil {
		t.Fatal(err)
	}
	jpeg := []byte{0xff, 0xd8, 0xff, 0x01}
	encoded := base64.StdEncoding.EncodeToString(jpeg)
	if data, err := groupJPEG(encoded, 4); err != nil || !reflect.DeepEqual(data, jpeg) {
		t.Fatalf("JPEG: %x %v", data, err)
	}
	for _, value := range []string{"", "not-base64", base64.StdEncoding.EncodeToString([]byte("PNG"))} {
		if _, err := groupJPEG(value, 4); err == nil {
			t.Errorf("accepted invalid JPEG %q", value)
		}
	}
	if _, err := groupJPEG(encoded, 3); err == nil {
		t.Fatal("accepted picture above configured limit")
	}
}

func TestGroupProjectionPreservesIdentityAndUnknownFields(t *testing.T) {
	group := &types.GroupInfo{
		JID: types.NewJID("120363001", types.GroupServer), OwnerJID: types.NewJID("9988", types.HiddenUserServer),
		GroupName: types.GroupName{Name: "Operations"}, GroupTopic: types.GroupTopic{Topic: "Runbook", TopicID: "v1"},
		GroupCreated: time.Unix(1700000000, 0), ParticipantCount: 6,
		GroupLinkedParent: types.GroupLinkedParent{LinkedParentJID: types.NewJID("120363002", types.GroupServer)},
		GroupEphemeral:    types.GroupEphemeral{IsEphemeral: true, DisappearingTimer: 86400},
		Participants:      []types.GroupParticipant{{JID: types.NewJID("9988", types.HiddenUserServer), PhoneNumber: types.NewJID("15551234567", types.DefaultUserServer), LID: types.NewJID("9988", types.HiddenUserServer), IsAdmin: true, IsSuperAdmin: true}},
	}
	view := groupView(group)
	for key, want := range map[string]any{"id": "120363001@g.us", "subject": "Operations", "description": "Runbook", "description_id": "v1", "creator": "9988@lid", "creation_time": int64(1700000000), "parent_group_id": "120363002@g.us", "participant_count": 6, "ephemeral_expiration_seconds": uint32(86400)} {
		if got := view[key]; !reflect.DeepEqual(got, want) {
			t.Errorf("%s = %#v, want %#v", key, got, want)
		}
	}
	for _, key := range []string{"is_general_chat", "no_frequently_forwarded", "allow_admin_reports", "group_history_enabled", "limit_sharing_enabled"} {
		if value, found := view[key]; !found || value != nil {
			t.Errorf("%s must report unknown, got %#v", key, value)
		}
	}
	participant := view["participants"].([]map[string]any)[0]
	if participant["participant_type"] != "superadmin" || participant["lid"] != "9988@lid" || participant["phone_number"] != "15551234567@s.whatsapp.net" {
		t.Fatalf("participant: %#v", participant)
	}
	empty := groupView(&types.GroupInfo{JID: group.JID})
	if empty["participant_count"] != nil || len(empty["participants"].([]map[string]any)) != 0 {
		t.Fatalf("unknown participant count must not be zero: %#v", empty)
	}
	group.ParticipantCount = 0
	if groupOverview(group)["participant_count"] != 1 {
		t.Fatal("known participant list count not used")
	}
}

func TestGroupParticipantChangesKeepErrorsAndInvites(t *testing.T) {
	changes := groupParticipantChanges([]types.GroupParticipant{{JID: types.NewJID("1234", types.HiddenUserServer)}, {JID: types.NewJID("5678", types.HiddenUserServer), Error: 403, AddRequest: &types.GroupParticipantAddRequest{Code: "invite-code", Expiration: time.Unix(1700000000, 0)}}})
	if changes[0]["error"] != nil || changes[0]["status"] != nil {
		t.Fatal("fabricated participant status")
	}
	if changes[1]["error"] != "403" || changes[1]["invite_code"] != "invite-code" || changes[1]["invite_expiration"] != int64(1700000000) {
		t.Fatalf("failure lost: %#v", changes[1])
	}
}

func TestGroupPictureOutcomes(t *testing.T) {
	for _, tc := range []struct {
		err    error
		status string
	}{{nil, "unchanged"}, {whatsmeow.ErrProfilePictureNotSet, "not_found"}, {whatsmeow.ErrProfilePictureUnauthorized, "not_authorized"}, {whatsmeow.ErrIQRateOverLimit, "rate_overlimit"}} {
		result, err := groupPictureView(nil, tc.err)
		if err != nil || result.(map[string]any)["status"] != tc.status {
			t.Fatalf("picture outcome: %#v %v", result, err)
		}
	}
	problem := errors.New("connection closed")
	if _, err := groupPictureView(nil, problem); !errors.Is(err, problem) {
		t.Fatalf("transport error hidden: %v", err)
	}
	result, err := groupPictureView(&types.ProfilePictureInfo{ID: "photo1", URL: "https://example.invalid/photo", DirectPath: "/photo", Hash: []byte{1, 2}}, nil)
	if err != nil || result.(map[string]any)["hash"] != "AQI=" || result.(map[string]any)["status"] != "found" {
		t.Fatalf("picture projection: %#v %v", result, err)
	}
}

func TestGroupCreateRequestAndUnsupportedOptions(t *testing.T) {
	req, err := groupCreateRequest(map[string]any{"subject": "Ops", "participants": []any{"15551234567@s.whatsapp.net"}}, false, false)
	if err != nil || req.Name != "Ops" || len(req.Participants) != 1 || req.IsParent {
		t.Fatalf("group create: %#v %v", req, err)
	}
	req, err = groupCreateRequest(map[string]any{"name": "Community", "description": "About"}, true, false)
	if err != nil || !req.IsParent {
		t.Fatalf("basic community create: %#v %v", req, err)
	}
	for _, field := range []string{"closed", "allow_non_admin_sub_group_creation", "create_general_chat"} {
		if _, err = groupCreateRequest(map[string]any{"name": "Community", field: true}, true, false); err == nil {
			t.Errorf("unsupported option %s ignored", field)
		}
	}
	req, err = groupCreateRequest(map[string]any{"name": "Subgroup", "community_id": "1234@g.us"}, false, true)
	if err != nil || req.LinkedParentJID.String() != "1234@g.us" {
		t.Fatalf("subgroup create: %#v %v", req, err)
	}
	if _, err = groupCreateRequest(map[string]any{"name": "Subgroup", "community_id": "1234@g.us", "hidden": true}, false, true); err == nil {
		t.Fatal("hidden subgroup option ignored")
	}
}

func TestContactSavePatchUsesContactSetSchema(t *testing.T) {
	patch, err := groupContactPatch(map[string]any{"jid": "15551234567@s.whatsapp.net", "full_name": "Alex Doe", "first_name": "Alex", "save_on_primary_addressbook": true})
	if err != nil {
		t.Fatal(err)
	}
	if patch.Type != appstate.WAPatchCriticalUnblockLow || len(patch.Mutations) != 1 {
		t.Fatalf("wrong patch: %#v", patch)
	}
	mutation := patch.Mutations[0]
	if mutation.Version != 2 || !reflect.DeepEqual(mutation.Index, []string{"contact", "15551234567@s.whatsapp.net"}) {
		t.Fatalf("wrong mutation schema: %#v", mutation)
	}
	action := mutation.Value.ContactAction
	if action.GetFullName() != "Alex Doe" || action.GetFirstName() != "Alex" || !action.GetSaveOnPrimaryAddressbook() {
		t.Fatalf("contact fields lost: %#v", action)
	}
	for _, args := range []map[string]any{{"jid": "1234@lid", "full_name": "Alex"}, {"jid": "15551234567:2@s.whatsapp.net", "full_name": "Alex"}, {"jid": "15551234567@s.whatsapp.net"}, {"jid": "15551234567@s.whatsapp.net", "full_name": " "}} {
		if _, err := groupContactPatch(args); err == nil {
			t.Errorf("invalid contact save accepted: %#v", args)
		}
	}
}

func TestContactProjectionsPreserveLIDAndDoNotInventBusiness(t *testing.T) {
	view := groupContactCheck(types.IsOnWhatsAppResponse{JID: types.NewJID("9988", types.HiddenUserServer), PhoneNumber: types.NewJID("15551234567", types.DefaultUserServer), IsIn: true})
	if view["lid"] != "9988@lid" || view["phone_number"] != "15551234567@s.whatsapp.net" || view["registered"] != true || view["is_business"] != nil {
		t.Fatalf("check: %#v", view)
	}
	info := groupContactInfo(types.NewJID("9988", types.HiddenUserServer), types.UserInfo{Devices: []types.JID{{Device: 4}, {Device: 7}}})
	if info["is_business"] != nil || !reflect.DeepEqual(info["devices"], []uint16{4, 7}) {
		t.Fatalf("info: %#v", info)
	}
}

func TestPrivacyCategoryValueValidation(t *testing.T) {
	for _, tc := range [][2]string{{"last", "contacts"}, {"profile", "contact_blacklist"}, {"readreceipts", "none"}, {"online", "match_last_seen"}, {"calladd", "known"}, {"defense", "off"}, {"stickers", "contact_allowlist"}} {
		if _, _, err := groupPrivacySetting(tc[0], tc[1]); err != nil {
			t.Errorf("%v: %v", tc, err)
		}
	}
	for _, tc := range [][2]string{{"last", "match_last_seen"}, {"readreceipts", "contacts"}, {"online", "none"}, {"defense", "all"}, {"unknown", "all"}, {"last", "unknown"}} {
		if _, _, err := groupPrivacySetting(tc[0], tc[1]); err == nil {
			t.Errorf("invalid privacy setting accepted: %v", tc)
		}
	}
	settings := groupPrivacyView(&types.PrivacySettings{LastSeen: types.PrivacySettingContacts, Defense: types.PrivacySettingOff})
	if len(settings) != 2 || settings[0]["category"] != "last" || settings[1]["category"] != "defense" {
		t.Fatalf("privacy projection invented unset values: %#v", settings)
	}
}

func TestCommunityLinkValidationHappensBeforeMutations(t *testing.T) {
	parent := types.NewJID("1234", types.GroupServer)
	valid := map[string]any{"subgroups": []any{map[string]any{"group_id": "5678@g.us"}}}
	if groups, err := groupLinkTargets(valid, parent, false); err != nil || len(groups) != 1 {
		t.Fatalf("valid link: %#v %v", groups, err)
	}
	for _, args := range []map[string]any{{"subgroups": []any{}}, {"subgroups": []any{map[string]any{"group_id": "1234@g.us"}}}, {"subgroups": []any{map[string]any{"group_id": "5678@g.us"}, map[string]any{"group_id": "5678@g.us"}}}, {"subgroups": []any{map[string]any{"group_id": "5678@g.us", "hidden": true}}}} {
		if _, err := groupLinkTargets(args, parent, false); err == nil {
			t.Errorf("invalid link accepted: %#v", args)
		}
	}
	if _, err := groupLinkTargets(map[string]any{"group_ids": []string{"5678@g.us"}, "remove_orphan_members": true}, parent, true); err == nil {
		t.Fatal("unsupported orphan removal ignored")
	}
}

func TestGroupRegistrationExcludesUnsupportedOperations(t *testing.T) {
	r := &Registry{handlers: make(map[string]operationHandler)}
	registerGroups(r)
	for _, id := range []string{"groups::list", "groups::get", "groups::create", "groups::join", "groups::approve_requests", "contacts::save", "profile::set_name", "privacy::get", "communities::create", "communities::create_subgroup", "communities::link_subgroups"} {
		if _, ok := r.handlers["whatsapp::"+id]; !ok {
			t.Errorf("missing supported handler %s", id)
		}
	}
	for _, id := range []string{"groups::remove_linked_participants", "groups::cancel_requests", "groups::revoke_request_codes", "groups::set_history", "groups::set_forwarding_restriction", "groups::set_admin_reports", "groups::set_limit_sharing", "groups::set_member_link_mode", "groups::set_member_history_mode", "contacts::remove", "privacy::update_disallowed", "communities::deactivate", "communities::join_subgroup", "communities::remove_participants"} {
		if _, ok := r.handlers["whatsapp::"+id]; ok {
			t.Errorf("unsupported handler registered: %s", id)
		}
	}
}

func TestGroupResponsesMatchDeclaredSchemas(t *testing.T) {
	group := groupView(&types.GroupInfo{JID: types.NewJID("1234", types.GroupServer)})
	for _, tc := range []struct {
		id    string
		value any
	}{
		{"groups::get", group}, {"groups::create", group}, {"groups::invite_info", group},
		{"groups::join", map[string]any{"group_id": "1234@g.us", "pending_approval": nil}},
		{"communities::create", group}, {"communities::create_subgroup", group}, {"communities::get_subgroup", group},
		{"communities::subgroups", []any{groupSubgroupView(&types.GroupLinkTarget{JID: types.NewJID("1234", types.GroupServer)})}},
		{"communities::subgroup_counts", []any{map[string]any{"group_id": "1234@g.us", "participant_count": nil}}},
		{"contacts::info", []any{groupContactInfo(types.NewJID("1234", types.HiddenUserServer), types.UserInfo{})}},
		{"contacts::check", []any{groupContactCheck(types.IsOnWhatsAppResponse{JID: types.NewJID("1234", types.HiddenUserServer)})}},
	} {
		t.Run(tc.id, func(t *testing.T) {
			compiler := validate.NewCompiler()
			uri := "https://whatsapp.invalid/response.json"
			if err := compiler.AddResource(uri, catalogEntries["whatsapp::"+tc.id].ResponseFormat); err != nil {
				t.Fatal(err)
			}
			schema, err := compiler.Compile(uri)
			if err != nil {
				t.Fatal(err)
			}
			data, err := json.Marshal(tc.value)
			if err != nil {
				t.Fatal(err)
			}
			var value any
			if err = json.Unmarshal(data, &value); err != nil {
				t.Fatal(err)
			}
			if err = schema.Validate(value); err != nil {
				t.Fatalf("response does not match declaration: %v", err)
			}
		})
	}
}
