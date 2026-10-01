package main

import (
	"context"
	"encoding/base64"
	"errors"
	"fmt"
	"sort"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/appstate"
	"go.mau.fi/whatsmeow/proto/waSyncAction"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

func groupJID(value string) (types.JID, error) {
	jid, err := parseJID(value)
	if err != nil {
		return jid, err
	}
	if jid.Server != types.GroupServer {
		return types.EmptyJID, errors.New("group_id must be a group JID")
	}
	return jid, nil
}

func contactJID(value string) (types.JID, error) {
	jid, err := parseJID(value)
	if err != nil {
		return jid, err
	}
	if jid.Server != types.DefaultUserServer && jid.Server != types.HiddenUserServer {
		return types.EmptyJID, errors.New("contact must be a phone-number or LID JID")
	}
	return jid, nil
}

func groupContacts(values []string, allowEmpty bool) ([]types.JID, error) {
	if (!allowEmpty && len(values) == 0) || len(values) > 1024 {
		return nil, errors.New("participants must contain 1 to 1024 unique contacts")
	}
	result := make([]types.JID, 0, len(values))
	seen := make(map[types.JID]bool, len(values))
	for _, value := range values {
		jid, err := contactJID(value)
		if err != nil {
			return nil, err
		}
		// Group mutations address a person, never a particular linked device.
		jid = jid.ToNonAD()
		if seen[jid] {
			return nil, errors.New("duplicate contact")
		}
		seen[jid] = true
		result = append(result, jid)
	}
	return result, nil
}

func groupText(value, field string, max int, allowEmpty bool) error {
	if !allowEmpty && strings.TrimSpace(value) == "" {
		return fmt.Errorf("%s must not be blank", field)
	}
	if !utf8.ValidString(value) || utf8.RuneCountInString(value) > max {
		return fmt.Errorf("%s exceeds %d characters", field, max)
	}
	return nil
}

func groupJPEG(value string, max int) ([]byte, error) {
	data, err := decodeMedia(value, max)
	if err != nil {
		return nil, err
	}
	if len(data) < 3 || data[0] != 0xff || data[1] != 0xd8 || data[2] != 0xff {
		return nil, errors.New("picture must contain prepared JPEG bytes")
	}
	return data, nil
}

func groupOptionalString(value string) any {
	if value == "" {
		return nil
	}
	return value
}

func groupOptionalJID(value types.JID) any {
	if value.IsEmpty() {
		return nil
	}
	return value.String()
}

func groupOptionalTime(value time.Time) any {
	if value.IsZero() || value.Unix() < 0 {
		return nil
	}
	return value.Unix()
}

func groupParticipantView(p types.GroupParticipant) map[string]any {
	role := "member"
	if p.IsSuperAdmin {
		role = "superadmin"
	} else if p.IsAdmin {
		role = "admin"
	}
	return map[string]any{"jid": p.JID.String(), "phone_number": groupOptionalJID(p.PhoneNumber), "lid": groupOptionalJID(p.LID), "username": nil, "participant_type": role}
}

func groupOverview(g *types.GroupInfo) map[string]any {
	var count any
	if g.ParticipantCount > 0 {
		count = g.ParticipantCount
	} else if len(g.Participants) > 0 {
		count = len(g.Participants)
	}
	return map[string]any{"id": g.JID.String(), "subject": groupOptionalString(g.Name), "participant_count": count, "is_community": g.IsParent, "parent_group_id": groupOptionalJID(g.LinkedParentJID)}
}

func groupView(g *types.GroupInfo) map[string]any {
	if g == nil {
		return nil
	}
	out := groupOverview(g)
	participants := make([]map[string]any, 0, len(g.Participants))
	for _, p := range g.Participants {
		participants = append(participants, groupParticipantView(p))
	}
	out["participants"] = participants
	out["description"] = groupOptionalString(g.Topic)
	out["description_id"] = groupOptionalString(g.TopicID)
	out["creator"] = groupOptionalJID(g.OwnerJID)
	out["creation_time"] = groupOptionalTime(g.GroupCreated)
	out["locked"] = g.IsLocked
	out["announcement"] = g.IsAnnounce
	out["ephemeral_expiration_seconds"] = nil
	if g.IsEphemeral {
		out["ephemeral_expiration_seconds"] = g.DisappearingTimer
	}
	out["membership_approval"] = g.IsJoinApprovalRequired
	out["member_add_mode"] = groupOptionalString(string(g.MemberAddMode))
	out["is_announcement_subgroup"] = g.IsDefaultSubGroup
	// The public whatsmeow model does not expose these fields. Null means unknown.
	for _, field := range []string{"member_link_mode", "member_share_history_mode", "is_general_chat", "no_frequently_forwarded", "allow_admin_reports", "group_history_enabled", "limit_sharing_enabled"} {
		out[field] = nil
	}
	return out
}

func groupParticipantChanges(participants []types.GroupParticipant) []map[string]any {
	result := make([]map[string]any, 0, len(participants))
	for _, p := range participants {
		out := map[string]any{"jid": p.JID.String(), "status": nil, "error": nil, "phone_number": groupOptionalJID(p.PhoneNumber), "username": nil, "invite_code": nil, "invite_expiration": nil}
		if p.Error != 0 {
			out["error"] = strconv.Itoa(p.Error)
		}
		if p.AddRequest != nil {
			out["invite_code"] = p.AddRequest.Code
			out["invite_expiration"] = groupOptionalTime(p.AddRequest.Expiration)
		}
		result = append(result, out)
	}
	return result
}

func groupPictureView(p *types.ProfilePictureInfo, err error) (any, error) {
	status := ""
	switch {
	case errors.Is(err, whatsmeow.ErrProfilePictureNotSet):
		status = "not_found"
	case errors.Is(err, whatsmeow.ErrProfilePictureUnauthorized):
		status = "not_authorized"
	case errors.Is(err, whatsmeow.ErrIQRateOverLimit):
		status = "rate_overlimit"
	case err != nil:
		return nil, err
	case p == nil:
		status = "unchanged"
	}
	if status != "" {
		return map[string]any{"status": status}, nil
	}
	var hash any
	if len(p.Hash) > 0 {
		hash = base64.StdEncoding.EncodeToString(p.Hash)
	}
	return map[string]any{"status": "found", "id": p.ID, "url": p.URL, "direct_path": groupOptionalString(p.DirectPath), "hash": hash}, nil
}

func groupCreateRequest(args map[string]any, community, subgroup bool) (whatsmeow.ReqCreateGroup, error) {
	var req whatsmeow.ReqCreateGroup
	field := "subject"
	if community || subgroup {
		field = "name"
	}
	req.Name = stringArg(args, field)
	if err := groupText(req.Name, field, 100, false); err != nil {
		return req, err
	}
	var err error
	req.Participants, err = groupContacts(stringsArg(args, "participants"), true)
	if err != nil {
		return req, err
	}
	if community {
		for _, option := range []string{"closed", "allow_non_admin_sub_group_creation", "create_general_chat"} {
			if boolArg(args, option) {
				return req, fmt.Errorf("%s is not supported by whatsmeow community creation", option)
			}
		}
		if err = groupText(stringArg(args, "description"), "description", 2048, true); err != nil {
			return req, err
		}
		req.IsParent = true
	}
	if subgroup {
		if boolArg(args, "hidden") {
			return req, errors.New("hidden subgroups are not supported by whatsmeow")
		}
		req.LinkedParentJID, err = groupJID(stringArg(args, "community_id"))
	}
	return req, err
}

func registerGroups(r *Registry) {
	// Only public, supported library operations are registered; unsupported IDs stay absent.
	for _, community := range []bool{false, true} {
		prefix := "whatsapp::groups::"
		if community {
			prefix = "whatsapp::communities::"
		}
		r.Add(prefix+"list", func(ctx context.Context, args map[string]any) (any, error) {
			cli, err := r.Manager.Client(stringArg(args, "session_id"))
			if err != nil {
				return nil, err
			}
			groups, err := cli.GetJoinedGroups(ctx)
			if err != nil {
				return nil, err
			}
			result := make([]map[string]any, 0, len(groups))
			for _, g := range groups {
				if g != nil && (!community || g.IsParent) {
					result = append(result, groupOverview(g))
				}
			}
			return result, nil
		})
		r.Add(prefix+"create", func(ctx context.Context, args map[string]any) (any, error) {
			req, err := groupCreateRequest(args, community, false)
			if err != nil {
				return nil, err
			}
			cli, err := r.Manager.Client(stringArg(args, "session_id"))
			if err != nil {
				return nil, err
			}
			group, err := cli.CreateGroup(ctx, req)
			if err != nil {
				return nil, err
			}
			if community && stringArg(args, "description") != "" {
				if err = cli.SetGroupTopic(ctx, group.JID, "", "", stringArg(args, "description")); err != nil {
					return nil, fmt.Errorf("community %s was created but setting its description failed; do not recreate it: %w", group.JID, err)
				}
				group.Topic = stringArg(args, "description")
			}
			return groupView(group), nil
		})
	}
	addGroup := func(id string, fn func(context.Context, *whatsmeow.Client, types.JID, map[string]any) (any, error)) {
		r.Add("whatsapp::groups::"+id, func(ctx context.Context, args map[string]any) (any, error) {
			jid, err := groupJID(stringArg(args, "group_id"))
			if err != nil {
				return nil, err
			}
			cli, err := r.Manager.Client(stringArg(args, "session_id"))
			if err != nil {
				return nil, err
			}
			return fn(ctx, cli, jid, args)
		})
	}
	addGroup("get", func(ctx context.Context, cli *whatsmeow.Client, jid types.JID, args map[string]any) (any, error) {
		group, err := cli.GetGroupInfo(ctx, jid)
		if err != nil {
			return nil, err
		}
		return groupView(group), nil
	})
	addGroup("set_subject", func(ctx context.Context, cli *whatsmeow.Client, jid types.JID, args map[string]any) (any, error) {
		subject := stringArg(args, "subject")
		if err := groupText(subject, "subject", 100, false); err != nil {
			return nil, err
		}
		return nil, cli.SetGroupName(ctx, jid, subject)
	})
	addGroup("set_description", func(ctx context.Context, cli *whatsmeow.Client, jid types.JID, args map[string]any) (any, error) {
		description := stringArg(args, "description")
		if err := groupText(description, "description", 2048, true); err != nil {
			return nil, err
		}
		previous := stringArg(args, "previous_description_id")
		if len(previous) > 1024 {
			return nil, errors.New("previous_description_id exceeds 1024 bytes")
		}
		return nil, cli.SetGroupTopic(ctx, jid, previous, "", description)
	})
	addGroup("leave", func(ctx context.Context, cli *whatsmeow.Client, jid types.JID, args map[string]any) (any, error) {
		return nil, cli.LeaveGroup(ctx, jid)
	})
	for id, action := range map[string]whatsmeow.ParticipantChange{"add_participants": whatsmeow.ParticipantChangeAdd, "remove_participants": whatsmeow.ParticipantChangeRemove, "promote_participants": whatsmeow.ParticipantChangePromote, "demote_participants": whatsmeow.ParticipantChangeDemote} {
		addGroup(id, func(ctx context.Context, cli *whatsmeow.Client, jid types.JID, args map[string]any) (any, error) {
			participants, err := groupContacts(stringsArg(args, "participants"), false)
			if err != nil {
				return nil, err
			}
			result, err := cli.UpdateGroupParticipants(ctx, jid, participants, action)
			if err != nil {
				return nil, err
			}
			return groupParticipantChanges(result), nil
		})
	}
	for id, action := range map[string]whatsmeow.ParticipantRequestChange{"approve_requests": whatsmeow.ParticipantChangeApprove, "reject_requests": whatsmeow.ParticipantChangeReject} {
		addGroup(id, func(ctx context.Context, cli *whatsmeow.Client, jid types.JID, args map[string]any) (any, error) {
			participants, err := groupContacts(stringsArg(args, "participants"), false)
			if err != nil {
				return nil, err
			}
			result, err := cli.UpdateGroupRequestParticipants(ctx, jid, participants, action)
			if err != nil {
				return nil, err
			}
			return groupParticipantChanges(result), nil
		})
	}
	addGroup("membership_requests", func(ctx context.Context, cli *whatsmeow.Client, jid types.JID, args map[string]any) (any, error) {
		participants, err := cli.GetGroupRequestParticipants(ctx, jid)
		if err != nil {
			return nil, err
		}
		result := make([]map[string]any, 0, len(participants))
		for _, p := range participants {
			result = append(result, map[string]any{"jid": p.JID.String(), "request_time": groupOptionalTime(p.RequestedAt)})
		}
		return result, nil
	})
	for _, join := range []bool{false, true} {
		id := "invite_info"
		if join {
			id = "join"
		}
		r.Add("whatsapp::groups::"+id, func(ctx context.Context, args map[string]any) (any, error) {
			code := stringArg(args, "code")
			if strings.TrimSpace(code) == "" || len(code) > 256 {
				return nil, errors.New("invite code must contain 1 to 256 bytes")
			}
			cli, err := r.Manager.Client(stringArg(args, "session_id"))
			if err != nil {
				return nil, err
			}
			if join {
				jid, err := cli.JoinGroupWithLink(ctx, code)
				if err != nil {
					return nil, err
				}
				// The public method returns the same JID for a join and a pending request.
				return map[string]any{"group_id": jid.String(), "pending_approval": nil}, nil
			}
			group, err := cli.GetGroupInfoFromLink(ctx, code)
			if err != nil {
				return nil, err
			}
			return groupView(group), nil
		})
	}
	for _, reset := range []bool{false, true} {
		id := "invite_link"
		if reset {
			id = "revoke_invite"
		}
		addGroup(id, func(ctx context.Context, cli *whatsmeow.Client, jid types.JID, args map[string]any) (any, error) {
			url, err := cli.GetGroupInviteLink(ctx, jid, reset)
			if err != nil {
				return nil, err
			}
			return map[string]any{"url": url}, nil
		})
	}
	for id, method := range map[string]func(*whatsmeow.Client, context.Context, types.JID, bool) error{"set_locked": (*whatsmeow.Client).SetGroupLocked, "set_announcement": (*whatsmeow.Client).SetGroupAnnounce, "set_membership_approval": (*whatsmeow.Client).SetGroupJoinApprovalMode} {
		addGroup(id, func(ctx context.Context, cli *whatsmeow.Client, jid types.JID, args map[string]any) (any, error) {
			return nil, method(cli, ctx, jid, boolArg(args, "enabled"))
		})
	}
	addGroup("set_ephemeral", func(ctx context.Context, cli *whatsmeow.Client, jid types.JID, args map[string]any) (any, error) {
		seconds := intArg(args, "expiration_seconds", 0)
		if seconds < 0 || uint64(seconds) > uint64(^uint32(0)) {
			return nil, errors.New("invalid expiration_seconds")
		}
		return nil, cli.SetDisappearingTimer(ctx, jid, time.Duration(seconds)*time.Second, time.Time{})
	})
	addGroup("set_member_add_mode", func(ctx context.Context, cli *whatsmeow.Client, jid types.JID, args map[string]any) (any, error) {
		mode := types.GroupMemberAddModeAdmin
		if boolArg(args, "all_members") {
			mode = types.GroupMemberAddModeAllMember
		}
		return nil, cli.SetGroupMemberAddMode(ctx, jid, mode)
	})
	for _, remove := range []bool{false, true} {
		id := "set_picture"
		if remove {
			id = "remove_picture"
		}
		addGroup(id, func(ctx context.Context, cli *whatsmeow.Client, jid types.JID, args map[string]any) (any, error) {
			var data []byte
			var err error
			if !remove {
				data, err = groupJPEG(stringArg(args, "jpeg_base64"), r.Manager.Config().MaxMediaBytes)
				if err != nil {
					return nil, err
				}
			}
			id, err := cli.SetGroupPhoto(ctx, jid, data)
			if err != nil {
				return nil, err
			}
			return map[string]any{"id": id}, nil
		})
	}
	for _, group := range []bool{false, true} {
		prefix := "contacts"
		if group {
			prefix = "groups"
		}
		r.Add("whatsapp::"+prefix+"::picture", func(ctx context.Context, args map[string]any) (any, error) {
			parse := contactJID
			if group {
				parse = groupJID
			}
			jid, err := parse(stringArg(args, "jid"))
			if err != nil {
				return nil, err
			}
			cli, err := r.Manager.Client(stringArg(args, "session_id"))
			if err != nil {
				return nil, err
			}
			params := &whatsmeow.GetProfilePictureParams{Preview: boolArg(args, "preview"), ExistingID: stringArg(args, "existing_id")}
			picture, err := cli.GetProfilePictureInfo(ctx, jid, params)
			if group && errors.Is(err, whatsmeow.ErrProfilePictureUnauthorized) {
				// Community pictures use a distinct namespace. Preserve the original
				// lookup result if metadata is unavailable (e.g. an unjoined group).
				if info, infoErr := cli.GetGroupInfo(ctx, jid); infoErr == nil && info.IsParent {
					params.IsCommunity = true
					picture, err = cli.GetProfilePictureInfo(ctx, jid, params)
				}
			}
			return groupPictureView(picture, err)
		})
	}
	registerGroupContacts(r)
	registerGroupProfile(r)
	registerCommunities(r)
}

func groupContactPatch(args map[string]any) (appstate.PatchInfo, error) {
	var patch appstate.PatchInfo
	jid, err := contactJID(stringArg(args, "jid"))
	if err != nil {
		return patch, err
	}
	if jid.Server != types.DefaultUserServer || jid.Device != 0 {
		return patch, errors.New("saved contact must be a bare phone-number JID")
	}
	full, hasFull := args["full_name"].(string)
	first, hasFirst := args["first_name"].(string)
	if !hasFull && !hasFirst {
		return patch, errors.New("full_name or first_name is required")
	}
	for _, name := range []struct {
		value string
		set   bool
	}{{full, hasFull}, {first, hasFirst}} {
		if name.set && (strings.TrimSpace(name.value) == "" || len(name.value) > 1024) {
			return patch, errors.New("contact name must contain 1 to 1024 bytes")
		}
	}
	action := &waSyncAction.ContactAction{SaveOnPrimaryAddressbook: proto.Bool(boolArg(args, "save_on_primary_addressbook"))}
	if hasFull {
		action.FullName = &full
	}
	if hasFirst {
		action.FirstName = &first
	}
	return appstate.PatchInfo{Type: appstate.WAPatchCriticalUnblockLow, Mutations: []appstate.MutationInfo{{Index: []string{appstate.IndexContact, jid.String()}, Version: 2, Value: &waSyncAction.SyncActionValue{ContactAction: action}}}}, nil
}

func registerGroupContacts(r *Registry) {
	r.Add("whatsapp::contacts::pictures", func(ctx context.Context, args map[string]any) (any, error) {
		values := stringsArg(args, "jids")
		if len(values) > 50 {
			return nil, errors.New("at most 50 contact photos per request")
		}
		jids, err := groupContacts(values, false)
		if err != nil {
			return nil, err
		}
		cli, err := r.Manager.Client(stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		result := make([]map[string]any, 0, len(jids))
		for _, jid := range jids {
			picture, fetchErr := cli.GetProfilePictureInfo(ctx, jid, nil)
			view, lookupErr := groupPictureView(picture, fetchErr)
			var problem any
			if lookupErr != nil {
				problem = lookupErr.Error()
			}
			result = append(result, map[string]any{"jid": jid.String(), "picture": view, "error": problem})
		}
		return result, nil
	})
	r.Add("whatsapp::contacts::resolve_phone", func(ctx context.Context, args map[string]any) (any, error) {
		jid, err := contactJID(stringArg(args, "jid"))
		if err != nil {
			return nil, err
		}
		cli, err := r.Manager.Client(stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		phone := jid
		if jid.Server == types.HiddenUserServer {
			phone, err = cli.Store.LIDs.GetPNForLID(ctx, jid.ToNonAD())
			if err != nil {
				return nil, err
			}
		}
		var digits any
		if phone.Server == types.DefaultUserServer {
			digits = phone.User
		}
		return map[string]any{"jid": jid.String(), "phone_number": digits}, nil
	})
	r.Add("whatsapp::contacts::check", func(ctx context.Context, args map[string]any) (any, error) {
		jids, err := groupContacts(stringsArg(args, "jids"), false)
		if err != nil {
			return nil, err
		}
		cli, err := r.Manager.Client(stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		phones := make([]string, 0, len(jids))
		for _, jid := range jids {
			if jid.Server == types.HiddenUserServer {
				jid, err = cli.Store.LIDs.GetPNForLID(ctx, jid)
				if err != nil {
					return nil, err
				}
				if jid.IsEmpty() {
					return nil, errors.New("checking an LID requires a known phone-number mapping")
				}
			}
			phones = append(phones, "+"+jid.User)
		}
		users, err := cli.IsOnWhatsApp(ctx, phones)
		if err != nil {
			return nil, err
		}
		result := make([]map[string]any, 0, len(users))
		for _, user := range users {
			result = append(result, groupContactCheck(user))
		}
		return result, nil
	})
	r.Add("whatsapp::contacts::info", func(ctx context.Context, args map[string]any) (any, error) {
		jids, err := groupContacts(stringsArg(args, "jids"), false)
		if err != nil {
			return nil, err
		}
		cli, err := r.Manager.Client(stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		users, err := cli.GetUserInfo(ctx, jids)
		if err != nil {
			return nil, err
		}
		result := make([]map[string]any, 0, len(users))
		for jid, user := range users {
			result = append(result, groupContactInfo(jid, user))
		}
		sort.Slice(result, func(i, j int) bool { return result[i]["jid"].(string) < result[j]["jid"].(string) })
		return result, nil
	})
	r.Add("whatsapp::contacts::save", func(ctx context.Context, args map[string]any) (any, error) {
		patch, err := groupContactPatch(args)
		if err != nil {
			return nil, err
		}
		cli, err := r.Manager.Client(stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		return nil, cli.SendAppState(ctx, patch)
	})
	r.Add("whatsapp::contacts::blocked", func(ctx context.Context, args map[string]any) (any, error) {
		cli, err := r.Manager.Client(stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		blocklist, err := cli.GetBlocklist(ctx)
		if err != nil {
			return nil, err
		}
		result := make([]map[string]any, 0, len(blocklist.JIDs))
		for _, jid := range blocklist.JIDs {
			result = append(result, map[string]any{"jid": jid.String(), "timestamp": nil})
		}
		return result, nil
	})
	r.Add("whatsapp::contacts::is_blocked", func(ctx context.Context, args map[string]any) (any, error) {
		jid, err := contactJID(stringArg(args, "jid"))
		if err != nil {
			return nil, err
		}
		jid = jid.ToNonAD()
		cli, err := r.Manager.Client(stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		alternate := types.EmptyJID
		if jid.Server == types.HiddenUserServer {
			alternate, err = cli.Store.LIDs.GetPNForLID(ctx, jid)
		} else {
			alternate, err = cli.Store.LIDs.GetLIDForPN(ctx, jid)
		}
		if err != nil {
			return nil, err
		}
		blocklist, err := cli.GetBlocklist(ctx)
		if err != nil {
			return nil, err
		}
		for _, blocked := range blocklist.JIDs {
			if blocked.ToNonAD() == jid || (!alternate.IsEmpty() && blocked.ToNonAD() == alternate) {
				return map[string]any{"blocked": true}, nil
			}
		}
		return map[string]any{"blocked": false}, nil
	})
	for _, block := range []bool{false, true} {
		id := "unblock"
		action := events.BlocklistChangeActionUnblock
		if block {
			id = "block"
			action = events.BlocklistChangeActionBlock
		}
		r.Add("whatsapp::contacts::"+id, func(ctx context.Context, args map[string]any) (any, error) {
			jid, err := contactJID(stringArg(args, "jid"))
			if err != nil {
				return nil, err
			}
			cli, err := r.Manager.Client(stringArg(args, "session_id"))
			if err != nil {
				return nil, err
			}
			_, err = cli.UpdateBlocklist(ctx, jid.ToNonAD(), action)
			return nil, err
		})
	}
}

func groupContactCheck(user types.IsOnWhatsAppResponse) map[string]any {
	var lid any
	if user.JID.Server == types.HiddenUserServer {
		lid = user.JID.String()
	}
	pn := user.PhoneNumber
	if pn.IsEmpty() && user.JID.Server == types.DefaultUserServer {
		pn = user.JID
	}
	var business any
	if user.VerifiedName != nil {
		business = true
	}
	return map[string]any{"jid": user.JID.String(), "lid": lid, "phone_number": groupOptionalJID(pn), "registered": user.IsIn, "is_business": business, "username": nil, "contact_error": nil, "lid_error": nil, "business_error": nil, "username_error": nil}
}

func groupContactInfo(jid types.JID, user types.UserInfo) map[string]any {
	devices := make([]uint16, 0, len(user.Devices))
	for _, device := range user.Devices {
		devices = append(devices, device.Device)
	}
	var business any
	if user.VerifiedName != nil {
		business = true
	}
	return map[string]any{"jid": jid.String(), "lid": groupOptionalJID(user.LID), "about": groupOptionalString(user.Status), "picture_id": groupOptionalString(user.PictureID), "is_business": business, "username": nil, "devices": devices, "lid_error": nil, "status_error": nil, "picture_error": nil, "business_error": nil, "username_error": nil, "devices_error": nil}
}

func groupPrivacySetting(category, value string) (types.PrivacySettingType, types.PrivacySetting, error) {
	valid := false
	switch category {
	case "last", "profile", "status", "groupadd":
		valid = value == "all" || value == "contacts" || value == "contact_blacklist" || value == "none"
	case "readreceipts":
		valid = value == "all" || value == "none"
	case "online":
		valid = value == "all" || value == "match_last_seen"
	case "calladd":
		valid = value == "all" || value == "known" || value == "contacts"
	case "messages":
		valid = value == "all" || value == "contacts"
	case "defense":
		valid = value == "off" || value == "on_standard"
	case "stickers":
		valid = value == "contacts" || value == "contact_allowlist" || value == "none"
	}
	if !valid {
		return "", "", errors.New("unsupported privacy category or invalid value for category")
	}
	return types.PrivacySettingType(category), types.PrivacySetting(value), nil
}

func groupPrivacyView(settings *types.PrivacySettings) []map[string]any {
	result := make([]map[string]any, 0, 10)
	for _, setting := range []struct {
		category string
		value    types.PrivacySetting
	}{{"groupadd", settings.GroupAdd}, {"last", settings.LastSeen}, {"status", settings.Status}, {"profile", settings.Profile}, {"readreceipts", settings.ReadReceipts}, {"online", settings.Online}, {"calladd", settings.CallAdd}, {"messages", settings.Messages}, {"defense", settings.Defense}, {"stickers", settings.Stickers}} {
		if setting.value != "" {
			result = append(result, map[string]any{"category": setting.category, "value": string(setting.value)})
		}
	}
	return result
}

func registerGroupProfile(r *Registry) {
	r.Add("whatsapp::profile::set_name", func(ctx context.Context, args map[string]any) (any, error) {
		name := stringArg(args, "text")
		if strings.TrimSpace(name) == "" || len(name) > 256 {
			return nil, errors.New("profile name must contain 1 to 256 bytes")
		}
		cli, err := r.Manager.Client(stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		return nil, cli.SendAppState(ctx, appstate.BuildSettingPushName(name))
	})
	r.Add("whatsapp::profile::set_about", func(ctx context.Context, args map[string]any) (any, error) {
		text := stringArg(args, "text")
		if len(text) > 4096 {
			return nil, errors.New("about exceeds 4096 bytes")
		}
		cli, err := r.Manager.Client(stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		return nil, cli.SetStatusMessage(ctx, types.SetStatusInput{Text: &text})
	})
	for _, remove := range []bool{false, true} {
		id := "set_picture"
		if remove {
			id = "remove_picture"
		}
		r.Add("whatsapp::profile::"+id, func(ctx context.Context, args map[string]any) (any, error) {
			var data []byte
			var err error
			if !remove {
				data, err = groupJPEG(stringArg(args, "jpeg_base64"), r.Manager.Config().MaxMediaBytes)
				if err != nil {
					return nil, err
				}
			}
			cli, err := r.Manager.Client(stringArg(args, "session_id"))
			if err != nil {
				return nil, err
			}
			if cli.Store.ID == nil {
				return nil, errors.New("session has no paired identity")
			}
			id, err := cli.SetGroupPhoto(ctx, cli.Store.ID.ToNonAD(), data)
			if err != nil {
				return nil, err
			}
			return map[string]any{"id": id}, nil
		})
	}
	r.Add("whatsapp::privacy::get", func(ctx context.Context, args map[string]any) (any, error) {
		cli, err := r.Manager.Client(stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		settings, err := cli.TryFetchPrivacySettings(ctx, true)
		if err != nil {
			return nil, err
		}
		return groupPrivacyView(settings), nil
	})
	r.Add("whatsapp::privacy::set", func(ctx context.Context, args map[string]any) (any, error) {
		category, value, err := groupPrivacySetting(stringArg(args, "category"), stringArg(args, "value"))
		if err != nil {
			return nil, err
		}
		cli, err := r.Manager.Client(stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		_, err = cli.SetPrivacySetting(ctx, category, value)
		if err != nil {
			return nil, err
		}
		return map[string]any{"dhash": nil}, nil
	})
	r.Add("whatsapp::privacy::set_disappearing_default", func(ctx context.Context, args map[string]any) (any, error) {
		seconds := intArg(args, "expiration_seconds", 0)
		if seconds < 0 || uint64(seconds) > uint64(^uint32(0)) {
			return nil, errors.New("invalid expiration_seconds")
		}
		cli, err := r.Manager.Client(stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		return nil, cli.SetDefaultDisappearingTimer(ctx, time.Duration(seconds)*time.Second)
	})
}

func groupSubgroupView(group *types.GroupLinkTarget) map[string]any {
	return map[string]any{"id": group.JID.String(), "subject": group.Name, "participant_count": nil, "creation_time": nil, "owner": nil, "is_announcement_subgroup": group.IsDefaultSubGroup, "is_general_chat": nil, "hidden": nil}
}

func groupLinkTargets(args map[string]any, parent types.JID, unlink bool) ([]types.JID, error) {
	var values []string
	if unlink {
		if boolArg(args, "remove_orphan_members") {
			return nil, errors.New("remove_orphan_members is not supported by whatsmeow")
		}
		values = stringsArg(args, "group_ids")
	} else {
		items, _ := args["subgroups"].([]any)
		for _, item := range items {
			value, ok := item.(map[string]any)
			if !ok {
				return nil, errors.New("invalid subgroup")
			}
			if boolArg(value, "hidden") {
				return nil, errors.New("hidden subgroups are not supported by whatsmeow")
			}
			values = append(values, stringArg(value, "group_id"))
		}
	}
	if len(values) == 0 || len(values) > 1024 {
		return nil, errors.New("subgroups must contain 1 to 1024 entries")
	}
	result := make([]types.JID, 0, len(values))
	seen := make(map[types.JID]bool, len(values))
	for _, value := range values {
		jid, err := groupJID(value)
		if err != nil {
			return nil, err
		}
		if jid == parent {
			return nil, errors.New("community cannot be its own subgroup")
		}
		if seen[jid] {
			return nil, errors.New("duplicate subgroup")
		}
		seen[jid] = true
		result = append(result, jid)
	}
	return result, nil
}

func registerCommunities(r *Registry) {
	addCommunity := func(id string, fn func(context.Context, *whatsmeow.Client, types.JID, map[string]any) (any, error)) {
		r.Add("whatsapp::communities::"+id, func(ctx context.Context, args map[string]any) (any, error) {
			jid, err := groupJID(stringArg(args, "community_id"))
			if err != nil {
				return nil, err
			}
			cli, err := r.Manager.Client(stringArg(args, "session_id"))
			if err != nil {
				return nil, err
			}
			return fn(ctx, cli, jid, args)
		})
	}
	r.Add("whatsapp::communities::create_subgroup", func(ctx context.Context, args map[string]any) (any, error) {
		req, err := groupCreateRequest(args, false, true)
		if err != nil {
			return nil, err
		}
		cli, err := r.Manager.Client(stringArg(args, "session_id"))
		if err != nil {
			return nil, err
		}
		group, err := cli.CreateGroup(ctx, req)
		if err != nil {
			return nil, err
		}
		return groupView(group), nil
	})
	addCommunity("subgroups", func(ctx context.Context, cli *whatsmeow.Client, jid types.JID, args map[string]any) (any, error) {
		groups, err := cli.GetSubGroups(ctx, jid)
		if err != nil {
			return nil, err
		}
		result := make([]map[string]any, 0, len(groups))
		for _, group := range groups {
			if group != nil {
				result = append(result, groupSubgroupView(group))
			}
		}
		return result, nil
	})
	addCommunity("subgroup_counts", func(ctx context.Context, cli *whatsmeow.Client, jid types.JID, args map[string]any) (any, error) {
		groups, err := cli.GetSubGroups(ctx, jid)
		if err != nil {
			return nil, err
		}
		result := make([]map[string]any, 0, len(groups))
		for _, group := range groups {
			if group == nil {
				continue
			}
			info, err := cli.GetGroupInfo(ctx, group.JID)
			if err != nil {
				return nil, err
			}
			result = append(result, map[string]any{"group_id": info.JID.String(), "participant_count": groupOverview(info)["participant_count"]})
		}
		return result, nil
	})
	addCommunity("get_subgroup", func(ctx context.Context, cli *whatsmeow.Client, parent types.JID, args map[string]any) (any, error) {
		jid, err := groupJID(stringArg(args, "subgroup_id"))
		if err != nil {
			return nil, err
		}
		group, err := cli.GetGroupInfo(ctx, jid)
		if err != nil {
			return nil, err
		}
		if group.LinkedParentJID != parent {
			return nil, errors.New("group is not a subgroup of the requested community")
		}
		return groupView(group), nil
	})
	for _, unlink := range []bool{false, true} {
		id := "link_subgroups"
		if unlink {
			id = "unlink_subgroups"
		}
		addCommunity(id, func(ctx context.Context, cli *whatsmeow.Client, parent types.JID, args map[string]any) (any, error) {
			targets, err := groupLinkTargets(args, parent, unlink)
			if err != nil {
				return nil, err
			}
			succeeded := make([]string, 0, len(targets))
			failed := make([]map[string]any, 0)
			for _, jid := range targets {
				if unlink {
					err = cli.UnlinkGroup(ctx, parent, jid)
				} else {
					err = cli.LinkGroup(ctx, parent, jid)
				}
				if err == nil {
					succeeded = append(succeeded, jid.String())
					continue
				}
				var iqErr *whatsmeow.IQError
				if errors.As(err, &iqErr) && iqErr.Code > 0 {
					failed = append(failed, map[string]any{"group_id": jid.String(), "code": iqErr.Code})
					continue
				}
				return nil, fmt.Errorf("community update stopped with an unconfirmed result for %s; confirmed successes: %v; inspect membership before retrying: %w", jid, succeeded, err)
			}
			return map[string]any{"succeeded": succeeded, "failed": failed}, nil
		})
	}
	addCommunity("participants", func(ctx context.Context, cli *whatsmeow.Client, parent types.JID, args map[string]any) (any, error) {
		jids, err := cli.GetLinkedGroupsParticipants(ctx, parent)
		if err != nil {
			return nil, err
		}
		result := make([]map[string]any, 0, len(jids))
		for _, jid := range jids {
			view := groupParticipantView(types.GroupParticipant{JID: jid})
			view["participant_type"] = "unknown"
			if jid.Server == types.HiddenUserServer {
				view["lid"] = jid.String()
			} else if jid.Server == types.DefaultUserServer {
				view["phone_number"] = jid.String()
			}
			result = append(result, view)
		}
		return result, nil
	})
}
