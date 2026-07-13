import { z } from 'zod'

export const CONSOLE_EXTENSION_API_VERSION = 1 as const
export const CONSOLE_EXTENSION_CAPABILITY = 'iii.console-extension' as const
export const CONSOLE_EXTENSION_FUNCTION_SUFFIX = '::console-extension'

const consoleExtensionFunctionInfoSchema = z.object({
  function_id: z.string(),
  metadata: z.object({
    internal: z.literal(true),
    capability: z.literal(CONSOLE_EXTENSION_CAPABILITY),
    api_version: z.literal(CONSOLE_EXTENSION_API_VERSION),
  }),
})

export function isConsoleExtensionCapability(
  value: unknown,
  functionId: string,
): boolean {
  const parsed = consoleExtensionFunctionInfoSchema.safeParse(value)
  return parsed.success && parsed.data.function_id === functionId
}

export const extensionAssetDescriptorSchema = z.object({
  path: z.string().min(1),
  media_type: z.string().min(1),
  etag: z.string().min(1),
})

export const consoleExtensionManifestSchema = z.object({
  id: z.string().min(1),
  api_version: z.number().int().positive(),
  worker_version: z.string().min(1),
  asset_function: z.string().min(1),
  entry: extensionAssetDescriptorSchema,
  styles: z.array(extensionAssetDescriptorSchema),
  slots: z.array(z.string().min(1)),
})
export type ConsoleExtensionManifest = z.infer<
  typeof consoleExtensionManifestSchema
>

export const consoleExtensionAssetSchema = z.object({
  path: z.string().min(1),
  media_type: z.string().min(1),
  encoding: z.literal('base64'),
  content: z.string(),
  etag: z.string().min(1),
})
export type ConsoleExtensionAsset = z.infer<typeof consoleExtensionAssetSchema>

export interface ConsoleExtensionDisposable {
  dispose(): void
}

export interface ConsoleExtensionSlotContribution {
  id: string
  slot: string
  order?: number
  mount(
    element: HTMLElement,
    context: Record<string, unknown>,
  ): undefined | (() => void) | ConsoleExtensionDisposable
}

export interface ConsoleExtensionHost {
  apiVersion: typeof CONSOLE_EXTENSION_API_VERSION
  extension: {
    id: string
    workerVersion: string
  }
  registerSlot(contribution: ConsoleExtensionSlotContribution): () => void
  trigger<T = unknown>(
    functionId: string,
    payload?: Record<string, unknown>,
  ): Promise<T>
  on(
    functionId: string,
    handler: (payload: unknown) => void | Promise<void>,
  ): () => void
  registerTrigger(input: {
    type: string
    function_id: string
    config: Record<string, unknown>
  }): () => void
  browserId: string
}

export interface ConsoleExtensionModule {
  activate(
    host: ConsoleExtensionHost,
  ):
    | undefined
    | ConsoleExtensionDisposable
    | Promise<undefined | ConsoleExtensionDisposable>
}

export function decodeBase64(content: string): Uint8Array {
  const binary = atob(content)
  const bytes = new Uint8Array(binary.length)
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i)
  }
  return bytes
}

export function contentEtag(bytes: Uint8Array): string {
  let hash = 0xcbf29ce484222325n
  for (const byte of bytes) {
    hash ^= BigInt(byte)
    hash = BigInt.asUintN(64, hash * 0x100000001b3n)
  }
  return `fnv1a64-${hash.toString(16).padStart(16, '0')}`
}

export function verifyExtensionAsset(
  descriptor: z.infer<typeof extensionAssetDescriptorSchema>,
  asset: ConsoleExtensionAsset,
): Uint8Array {
  if (asset.path !== descriptor.path) {
    throw new Error(
      `console extension asset path mismatch: expected ${descriptor.path}, got ${asset.path}`,
    )
  }
  if (asset.media_type !== descriptor.media_type) {
    throw new Error(
      `console extension asset media type mismatch for ${descriptor.path}`,
    )
  }
  if (asset.etag !== descriptor.etag) {
    throw new Error(
      `console extension asset etag mismatch for ${descriptor.path}`,
    )
  }
  const bytes = decodeBase64(asset.content)
  if (contentEtag(bytes) !== descriptor.etag) {
    throw new Error(
      `console extension asset content verification failed for ${descriptor.path}`,
    )
  }
  return bytes
}
