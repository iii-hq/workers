/** Screen-reader text for approval events. Names the call namespace with the
 * same wording as the visible prompt (`<subject> in namespace <ns>`). */
const inNamespace = (subject: string, namespace?: string) =>
  namespace ? `${subject} in namespace ${namespace}` : subject

export const actionRequiredText = (
  functionId: string,
  namespace?: string,
  requestedRoot?: string,
) =>
  requestedRoot
    ? `Action required: ${inNamespace(functionId, namespace)} needs approval to access ${requestedRoot}.`
    : `Action required: approve or deny ${inNamespace(functionId, namespace)}.`

export const approvedAlwaysText = (functionId: string, namespace?: string) =>
  `approved always this session: ${inNamespace(functionId, namespace)}`

/** Filesystem-access outcome ("allowed /x for this session") plus the call's
 * namespace. A folder decision is not per namespace, so the namespace names
 * the call, never the scope of the decision. */
export const filesystemDecisionText = (outcome: string, namespace?: string) =>
  namespace ? `${outcome}; the call is in namespace ${namespace}` : outcome
