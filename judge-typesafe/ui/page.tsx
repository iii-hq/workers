import type { Host, SecretKeyFieldProps } from '@iii-dev/console-ui'
import type { ComponentType } from 'react'
import { createJevConfigForm } from './src/configuration'

export default function setup(host: Host) {
  // The Console's shared key field, on Consoles that have it: keys go to the
  // secrets worker and the entry keeps `secret://TYPESAFE_API_KEY`.
  const secretField = host.components?.SecretKeyField as ComponentType<SecretKeyFieldProps> | undefined
  host.configForms.register('judge-typesafe', createJevConfigForm(host.iii, secretField))
}
