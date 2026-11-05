import type { HttpRequest as ApiRequest, HttpResponse as ApiResponse } from '@iii-dev/helpers/http'
import { iii } from './iii'

// biome-ignore lint/suspicious/noExplicitAny: generic default requires any for handler flexibility
export const useApi = <TBody = any>(
  config: {
    api_path: string
    http_method: string
    description?: string
    metadata?: Record<string, unknown>
  },
  handler: (req: ApiRequest<TBody>, logger: Console) => Promise<ApiResponse>,
) => {
  const function_id = `api::${config.http_method.toLowerCase()}::${config.api_path}`
  const logger = console

  iii.registerFunction(function_id, (req: ApiRequest<TBody>) => handler(req, logger), {
    description: config.description,
    metadata: config.metadata,
  })
  observeRoute(function_id, config.http_method, config.api_path)
}

/** Serves `method path` with `function_id`: the http worker routes each matching request to it. */
export const observeRoute = (function_id: string, method: string, path: string) =>
  iii.observe({ function_id: 'http::on-request', args: { method, path } }, function_id)
