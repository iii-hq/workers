// Shim for `storybook/test`, `@storybook/test`, `storybook/actions` and
// `@storybook/addon-actions`. `fn()` records calls into the actions log the
// runtime exposes; the interaction helpers are inert (play functions are not
// run by the stories worker).
const log = (globalThis.__storiesActions ||= [])

export function fn(implementation) {
  const mock = (...args) => {
    log.push({ name: mock.mockName || implementation?.name || 'fn', args: safe(args), at: Date.now() })
    return implementation ? implementation(...args) : undefined
  }
  mock.mockName = implementation?.name || 'fn'
  mock.mock = { calls: [] }
  mock.mockImplementation = (next) => {
    implementation = next
    return mock
  }
  mock.mockReturnValue = (value) => {
    implementation = () => value
    return mock
  }
  mock.mockResolvedValue = (value) => {
    implementation = () => Promise.resolve(value)
    return mock
  }
  mock.mockClear = () => {
    mock.mock.calls = []
    return mock
  }
  return mock
}

export function action(name) {
  return (...args) => log.push({ name, args: safe(args), at: Date.now() })
}

function safe(args) {
  try {
    return JSON.parse(
      JSON.stringify(args, (_, value) => {
        if (typeof value === 'function') return '[fn]'
        if (value && typeof value === 'object' && 'nativeEvent' in value) return `[event ${value.type}]`
        return value
      }),
    )
  } catch {
    return ['[unserializable]']
  }
}

const inert = () => Promise.resolve()
export const expect = () => new Proxy({}, { get: () => inert })
export const userEvent = new Proxy({}, { get: () => inert })
export const within = () => new Proxy({}, { get: () => inert })
export const waitFor = (cb) => Promise.resolve(cb?.())
export const screen = new Proxy({}, { get: () => inert })
export const spyOn = () => fn()
export const mocked = (value) => value
export const isMockFunction = (value) => typeof value === 'function' && 'mock' in value
export const clearAllMocks = () => {}
export default { fn, action, expect, userEvent, within, waitFor, screen }
