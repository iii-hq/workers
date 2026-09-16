/**
 * Clipboard writes that survive insecure origins (`http://<LAN-IP>`, where
 * `navigator.clipboard` is undefined). The implementation is the shared
 * `@iii-dev/console-ui/format` one so workers and the Console copy alike.
 */
export { copyText as copyTextToClipboard } from '@iii-dev/console-ui/format'
