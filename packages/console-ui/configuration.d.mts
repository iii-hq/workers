import type { ExtensionIii } from './index.js'
/** Returns the addressed worker's entry ID; rejects missing/invalid identities rather than selecting another project. */
export declare function resolveConfigurationId(iii: Pick<ExtensionIii, 'trigger'>, worker: string): Promise<string>
