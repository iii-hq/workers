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

/** Filesystem-access outcome ("allowed /x for this session") plus the namespace. */
export const filesystemDecisionText = (outcome: string, namespace?: string) =>
  namespace ? `${outcome} in namespace ${namespace}` : outcome
