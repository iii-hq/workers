/**
 * The github page shell (#/ext/github): two tabs over the console's shared
 * Tabs primitive.
 *
 * - graph (default, the hero): a live commit DAG of the working repo — colored
 *   lanes, merge routing, ref labels — refreshing as the agent works (GitGraph).
 * - activity: the existing live feed of `github::called` events, unchanged
 *   (ActivityFeed, from ./index).
 *
 * Radix Tabs unmounts the inactive tab's content, so each tab owns its own
 * `github::called` subscription only while visible — the two never run at once.
 */

import { type Host, Tabs, TabsContent, TabsList, TabsTrigger } from '@iii-dev/console-ui'
import { GitGraph } from './GitGraph'
import { ActivityFeed } from './index'

export function GithubPage({ host }: { host: Host }) {
  return (
    <Tabs defaultValue="graph" className="gh-tabs">
      <TabsList className="gh-tabs-list">
        <TabsTrigger value="graph">graph</TabsTrigger>
        <TabsTrigger value="activity">activity</TabsTrigger>
      </TabsList>
      <TabsContent value="graph" className="gh-tabs-content">
        <GitGraph host={host} />
      </TabsContent>
      <TabsContent value="activity" className="gh-tabs-content">
        <ActivityFeed host={host} />
      </TabsContent>
    </Tabs>
  )
}
