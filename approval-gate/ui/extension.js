const stores = new Map()

function node(tag, attrs = {}, children = []) {
  const element = document.createElement(tag)
  for (const [key, value] of Object.entries(attrs)) {
    if (value === undefined || value === null || value === false) continue
    if (key === 'className') element.className = value
    else if (key === 'text') element.textContent = value
    else if (key.startsWith('on') && typeof value === 'function') {
      element.addEventListener(key.slice(2).toLowerCase(), value)
    } else if (key in element && key !== 'style') {
      element[key] = value
    } else {
      element.setAttribute(key, String(value))
    }
  }
  for (const child of Array.isArray(children) ? children : [children]) {
    if (child === undefined || child === null || child === false) continue
    element.append(child instanceof Node ? child : document.createTextNode(String(child)))
  }
  return element
}

function button(label, onClick, variant = 'secondary') {
  return node('button', {
    type: 'button',
    className: `approval-extension__button approval-extension__button--${variant}`,
    text: label,
    onClick,
  })
}

function coerceSettings(raw) {
  const outer = raw && typeof raw === 'object' ? raw : {}
  const value = outer.settings && typeof outer.settings === 'object' ? outer.settings : outer
  return {
    mode: value.mode === 'auto' || value.mode === 'full' ? value.mode : 'manual',
    alwaysAllow: Array.isArray(value.always_allow) ? value.always_allow : [],
    approvedAlways: Array.isArray(value.approved_always) ? value.approved_always : [],
    modeSetAt: typeof value.mode_set_at === 'number' ? value.mode_set_at : 0,
  }
}

function sessionStore(host, sessionId) {
  let store = stores.get(sessionId)
  if (store) return store
  const listeners = new Set()
  store = {
    sessionId,
    loaded: false,
    loading: false,
    settings: coerceSettings(null),
    notify() {
      for (const listener of listeners) listener(store)
    },
    subscribe(listener) {
      listeners.add(listener)
      listener(store)
      return () => listeners.delete(listener)
    },
    async load() {
      if (store.loaded || store.loading) return
      store.loading = true
      store.notify()
      try {
        store.settings = coerceSettings(
          await host.trigger('approval::get-settings', { session_id: sessionId }),
        )
      } catch (error) {
        console.error('[approval-extension] get-settings failed', error)
      } finally {
        store.loaded = true
        store.loading = false
        store.notify()
      }
    },
    async setMode(mode) {
      const previous = store.settings
      store.settings = { ...previous, mode, modeSetAt: Date.now() }
      store.notify()
      try {
        store.settings = coerceSettings(
          await host.trigger('approval::set-mode', {
            session_id: sessionId,
            mode,
          }),
        )
      } catch (error) {
        store.settings = previous
        console.error('[approval-extension] set-mode failed', error)
      }
      store.notify()
    },
  }
  stores.set(sessionId, store)
  return store
}

function bindContext(element, initialContext, bind) {
  let cleanup = bind(initialContext)
  const onContext = (event) => {
    cleanup?.()
    element.replaceChildren()
    cleanup = bind(event.detail ?? {})
  }
  element.addEventListener('iii:console-extension-context', onContext)
  return () => {
    cleanup?.()
    element.removeEventListener('iii:console-extension-context', onContext)
  }
}

function permissionSelect(store, disabled) {
  const select = node('select', {
    className: 'approval-extension__select',
    'aria-label': 'approval mode',
    disabled,
  })
  for (const [value, label, title] of [
    ['manual', 'manual', 'pause every function until you approve or deny it'],
    ['auto', 'auto', 'automatically run functions on the configured allowlist'],
    ['full', 'full', 'run every function without asking'],
  ]) {
    select.append(node('option', { value, text: label, title }))
  }
  select.value = store.settings.mode
  select.addEventListener('change', () => {
    const mode = select.value
    if (
      mode === 'full' &&
      !window.confirm(
        'Enable full permissions? The agent will run every function without asking, including shell commands and file writes.',
      )
    ) {
      select.value = store.settings.mode
      return
    }
    void store.setMode(mode)
  })
  return select
}

function mountComposerControl(host, element, initialContext) {
  return bindContext(element, initialContext, (context) => {
    const sessionId = typeof context.sessionId === 'string' ? context.sessionId : null
    if (!sessionId) return
    const store = sessionStore(host, sessionId)
    const render = () => {
      element.replaceChildren(permissionSelect(store, Boolean(context.disabled) || !store.loaded))
    }
    const off = store.subscribe(render)
    void store.load()
    return off
  })
}

function mountBanner(host, element, initialContext) {
  return bindContext(element, initialContext, (context) => {
    const sessionId = typeof context.sessionId === 'string' ? context.sessionId : null
    if (!sessionId) return
    const store = sessionStore(host, sessionId)
    const render = () => {
      element.replaceChildren()
      if (store.settings.mode !== 'full') return
      const copy = node('p', {}, [
        node('strong', { text: 'full permissions active' }),
        node('span', {
          text: ' the agent runs every function without asking — including writing files, executing shells, and sending messages.',
        }),
      ])
      element.append(
        node('div', { className: 'approval-extension__banner', role: 'status' }, [
          copy,
          button('disable', () => void store.setMode('manual'), 'primary'),
        ]),
      )
    }
    const off = store.subscribe(render)
    void store.load()
    return off
  })
}

const destructivePattern = /(write|delete|remove|exec|run|send|credential|secret|chmod|move|rename)/i

function mountPendingActions(host, element, initialContext) {
  return bindContext(element, initialContext, (context) => {
    const message = context.message && typeof context.message === 'object' ? context.message : {}
    const sessionId = message.sessionId
    const functionCallId = message.functionCallId
    if (typeof sessionId !== 'string' || typeof functionCallId !== 'string') {
      element.append(
        node('div', {
          className: 'approval-extension__pending approval-extension__error',
          text: 'this approval is missing its session or function-call id.',
        }),
      )
      return
    }

    const filesystemAccess = message.filesystemAccess
    let submitting = false
    const container = node('div', { className: 'approval-extension__pending' })
    const actions = node('div', { className: 'approval-extension__actions' })
    const status = node('span', { className: 'approval-extension__status' })
    const error = node('div', { className: 'approval-extension__error' })

    const setBusy = (label) => {
      submitting = true
      status.textContent = label
      for (const control of actions.querySelectorAll('button')) control.disabled = true
    }
    const reset = (reason) => {
      submitting = false
      status.textContent = ''
      error.textContent = reason
      for (const control of actions.querySelectorAll('button')) control.disabled = false
    }
    const resolve = async (decision, accessDuration) => {
      if (submitting) return
      setBusy(decision === 'deny' ? 'denying…' : 'approving…')
      try {
        await host.trigger('approval::resolve', {
          session_id: sessionId,
          function_call_id: functionCallId,
          decision,
          ...(accessDuration ? { access_duration: accessDuration } : {}),
        })
        status.textContent = 'saved; waiting for the function to resume…'
      } catch (reason) {
        reset(reason instanceof Error ? reason.message : String(reason))
      }
    }

    if (filesystemAccess && typeof filesystemAccess.requestedRoot === 'string') {
      const root = filesystemAccess.requestedRoot
      container.append(
        node('code', { className: 'approval-extension__path', text: root, title: root }),
        node('p', {
          text: 'the function reached this folder and paused before accessing it. choose how long to allow access.',
        }),
      )
      actions.append(
        button('allow once', () => void resolve('allow', 'once'), 'primary'),
        button('allow this session', () => void resolve('allow', 'session')),
        button('always allow…', () => {
          if (
            window.confirm(
              `Always allow ${root}? This adds it to shell fs.host_roots for every conversation.`,
            )
          ) {
            void resolve('allow', 'always')
          }
        }),
        button('deny', () => void resolve('deny')),
      )
      const manage = button('manage filesystem access…', () => {
        window.dispatchEvent(
          new CustomEvent('approval-gate:open-filesystem-access', {
            detail: { sessionId },
          }),
        )
      }, 'link')
      container.append(actions, status, error, manage)
    } else {
      container.append(
        node('p', { text: 'execution is paused until you approve or deny this call.' }),
      )
      actions.append(
        button('approve', () => void resolve('allow'), 'primary'),
        button('deny', () => void resolve('deny')),
        button('approve always', async () => {
          if (
            destructivePattern.test(String(message.functionId ?? '')) &&
            !window.confirm(
              `Approve ${message.functionId} for the rest of this conversation without further prompts?`,
            )
          ) {
            return
          }
          if (submitting) return
          setBusy('saving…')
          try {
            await host.trigger('approval::approve-always', {
              session_id: sessionId,
              function_id: String(message.functionId ?? ''),
            })
            await host.trigger('approval::resolve', {
              session_id: sessionId,
              function_call_id: functionCallId,
              decision: 'allow',
            })
            status.textContent = 'saved; waiting for the function to resume…'
          } catch (reason) {
            reset(reason instanceof Error ? reason.message : String(reason))
          }
        }),
      )
      container.append(actions, status, error)
    }
    element.append(container)
  })
}

function structuredRule(entry) {
  return entry && typeof entry === 'object' && !Array.isArray(entry) ? entry : null
}

function autoAllowlist(rules) {
  return new Set(
    (Array.isArray(rules) ? rules : [])
      .map(structuredRule)
      .filter(
        (rule) =>
          rule &&
          rule.action === 'allow' &&
          Array.isArray(rule.modes) &&
          rule.modes.includes('auto') &&
          typeof rule.function === 'string',
      )
      .map((rule) => rule.function),
  )
}

function withoutAutoRules(rules) {
  return (Array.isArray(rules) ? rules : []).filter((entry) => {
    const rule = structuredRule(entry)
    return !(
      rule &&
      rule.action === 'allow' &&
      Array.isArray(rule.modes) &&
      rule.modes.length === 1 &&
      rule.modes[0] === 'auto'
    )
  })
}

async function readConfiguration(host, id) {
  const response = await host.trigger('configuration::get', { id, raw: true })
  return response && typeof response.value === 'object' && response.value ? response.value : {}
}

async function mountSettings(host, element) {
  const root = node('div', { className: 'approval-extension approval-extension__settings' })
  element.append(root)
  let config = {}
  let functions = []
  let allowlistOpen = false

  try {
    const [nextConfig, catalog] = await Promise.all([
      readConfiguration(host, 'approval-gate'),
      host.trigger('engine::functions::list', {}),
    ])
    config = nextConfig
    functions = Array.isArray(catalog.functions)
      ? catalog.functions
          .map((entry) => entry.function_id)
          .filter(
            (id) =>
              typeof id === 'string' &&
              !id.startsWith('approval::') &&
              !id.startsWith('configuration::'),
          )
          .sort()
      : []
  } catch (error) {
    root.append(node('p', { className: 'approval-extension__error', text: String(error) }))
    return
  }

  const save = async (defaultMode, allowlist) => {
    const rules = [
      ...withoutAutoRules(config.rules),
      ...[...allowlist].map((functionId) => ({
        function: functionId,
        action: 'allow',
        modes: ['auto'],
      })),
    ]
    config = { ...config, default_mode: defaultMode, rules }
    await host.trigger('configuration::set', {
      id: 'approval-gate',
      value: config,
    })
  }

  const render = () => {
    root.replaceChildren()
    const mode = config.default_mode === 'auto' || config.default_mode === 'full' ? config.default_mode : 'manual'
    const allowlist = autoAllowlist(config.rules)
    const modeSelect = permissionSelect(
      {
        settings: { mode },
        setMode: async (next) => {
          await save(next, allowlist)
          render()
        },
      },
      false,
    )
    const controls = [
      node('div', { className: 'approval-extension__settings-row' }, [
        node('span', { text: 'default mode' }),
        node('small', {
          text: 'manual prompts for everything · auto uses the allowlist · full skips prompts',
        }),
        modeSelect,
      ]),
    ]
    if (mode === 'auto') {
      controls.push(
        node('div', { className: 'approval-extension__settings-row' }, [
          node('span', { text: 'allowlist' }),
          node('small', { text: 'functions trusted automatically for new conversations' }),
          button(
            allowlistOpen ? 'close' : `manage${allowlist.size ? ` (${allowlist.size})` : ''}`,
            () => {
              allowlistOpen = !allowlistOpen
              render()
            },
          ),
        ]),
      )
    }
    const section = node('section', {}, [
      node('h2', { text: 'permissions' }),
      node('p', { text: 'defaults stored in the approval-gate configuration entry. applies to new conversations only.' }),
      node('div', { className: 'approval-extension__settings-rows' }, controls),
    ])
    if (allowlistOpen && mode === 'auto') {
      const list = node('div', { className: 'approval-extension__allowlist' })
      for (const functionId of functions) {
        const checkbox = node('input', { type: 'checkbox', checked: allowlist.has(functionId) })
        checkbox.addEventListener('change', async () => {
          if (checkbox.checked) allowlist.add(functionId)
          else allowlist.delete(functionId)
          await save(mode, allowlist)
        })
        list.append(node('label', {}, [checkbox, node('code', { text: functionId })]))
      }
      section.append(list)
    }
    root.append(
      section,
      node('section', {}, [
        node('h2', { text: 'filesystem access' }),
        node('p', {
          text: "the chosen workspace is always available; access outside it is approved from the function card.",
        }),
        node('a', {
          href: '#/workers/configuration/shell/fs/host_roots',
          className: 'approval-extension__button approval-extension__button--secondary',
          text: 'edit permanent roots',
        }),
      ]),
    )
  }
  render()
}

function mountWorkspaceAccess(host, element, initialContext) {
  return bindContext(element, initialContext, (context) => {
    const sessionId = typeof context.sessionId === 'string' ? context.sessionId : null
    if (!sessionId) return
    const trigger = button('access: workspace', () => void openDialog(), 'link')
    const dialog = node('dialog', { className: 'approval-extension__dialog' })
    element.append(trigger, dialog)

    const openDialog = async () => {
      dialog.replaceChildren(node('p', { text: 'loading filesystem access…' }))
      if (!dialog.open) dialog.showModal()
      const [grantResponse, shellConfig] = await Promise.all([
        host
          .trigger('harness::filesystem::grants', { session_id: sessionId })
          .catch(() => ({ roots: [] })),
        readConfiguration(host, 'shell').catch(() => ({})),
      ])
      const grants = Array.isArray(grantResponse.roots) ? grantResponse.roots : []
      const permanent = Array.isArray(shellConfig.fs?.host_roots) ? shellConfig.fs.host_roots : []
      const render = () => {
        const close = button('close', () => dialog.close())
        const groups = node('div', { className: 'approval-extension__folder-groups' })
        groups.append(folderGroup('workspace', context.workingDir ? [context.workingDir] : []))
        const sessionGroup = folderGroup('allowed this session', grants, async (root) => {
          await host.trigger('harness::filesystem::revoke', {
            session_id: sessionId,
            root,
          })
          grants.splice(grants.indexOf(root), 1)
          render()
        })
        groups.append(sessionGroup, folderGroup('always allowed', permanent))
        dialog.replaceChildren(
          node('header', {}, [node('h2', { text: 'filesystem access' }), close]),
          node('p', { text: 'folders the agent can read and write in this conversation.' }),
          context.sessionBusy
            ? node('p', { className: 'approval-extension__status', text: 'the agent is running; revoked access may be requested again.' })
            : null,
          groups,
          node('a', {
            href: '#/workers/configuration/shell/fs/host_roots',
            text: 'edit permanent roots →',
          }),
        )
      }
      render()
    }
    const onOpen = (event) => {
      if (event.detail?.sessionId === sessionId) void openDialog()
    }
    window.addEventListener('approval-gate:open-filesystem-access', onOpen)
    return () => window.removeEventListener('approval-gate:open-filesystem-access', onOpen)
  })
}

function folderGroup(title, roots, onRemove) {
  const content = node('div', { className: 'approval-extension__folder-list' })
  if (roots.length === 0) content.append(node('span', { text: 'none' }))
  for (const root of roots) {
    const children = [node('code', { text: root, title: root })]
    if (onRemove) children.push(button('revoke', () => void onRemove(root), 'link'))
    content.append(node('div', {}, children))
  }
  return node('section', {}, [node('h3', { text: title }), content])
}

export function activate(host) {
  if (host.apiVersion !== 1) {
    throw new Error(
      `approval-gate requires console extension API v1, got ${host.apiVersion}`,
    )
  }
  const disposers = [
    host.registerSlot({
      id: 'approval-gate.composer-mode',
      slot: 'chat.composer.controls',
      mount: (element, context) => mountComposerControl(host, element, context),
    }),
    host.registerSlot({
      id: 'approval-gate.full-banner',
      slot: 'chat.banner',
      mount: (element, context) => mountBanner(host, element, context),
    }),
    host.registerSlot({
      id: 'approval-gate.pending-actions',
      slot: 'function-call.pending-actions',
      mount: (element, context) => mountPendingActions(host, element, context),
    }),
    host.registerSlot({
      id: 'approval-gate.settings',
      slot: 'settings.sections',
      mount: (element) => {
        void mountSettings(host, element)
      },
    }),
    host.registerSlot({
      id: 'approval-gate.workspace-access',
      slot: 'chat.workspace-access',
      mount: (element, context) => mountWorkspaceAccess(host, element, context),
    }),
  ]
  return {
    dispose() {
      for (const dispose of disposers.reverse()) dispose()
      stores.clear()
    },
  }
}
