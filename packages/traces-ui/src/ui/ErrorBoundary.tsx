import { Component, type ReactNode } from 'react'
import { StatusPanel } from './StatusPanel'

interface State {
  error: Error | null
}

interface Props {
  children: ReactNode
  fallback?: (error: Error) => ReactNode
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null }

  static getDerivedStateFromError(error: Error): State {
    return { error }
  }

  componentDidCatch(error: Error) {
    console.error('[ErrorBoundary]', error)
  }

  render() {
    if (this.state.error) {
      if (this.props.fallback) return this.props.fallback(this.state.error)
      return (
        <StatusPanel
          variant="alert"
          headline="something broke"
          detail={this.state.error.message}
        />
      )
    }
    return this.props.children
  }
}
