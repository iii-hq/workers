export { errorAt, pointer } from './pointers'

export function FieldError({ message }: { message: string | null }) {
  if (!message) return null
  return (
    <div className="llmr-cfg-field-error" role="alert">
      {message}
    </div>
  )
}
