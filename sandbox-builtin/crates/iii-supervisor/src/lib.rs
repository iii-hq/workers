// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! The host↔guest channels of a sandbox-builtin VM, shared by iii-init
//! (guest) and sandbox-builtin (host): the worker env frame, the
//! shell-exec protocol, source sync, and the guest's lookup of the
//! virtio-console ports that carry them.

pub mod env_channel;
pub mod shell_protocol;
pub mod source_sync;
pub mod virtio;
