import type { Meta, StoryObj } from '@storybook/react-vite'
import { RadioTower } from 'lucide-react'
import { useState } from 'react'
import { ChatView } from '@/components/chat/ChatView'
import { BottomSheet, BottomSheetContent } from '@/components/ui/BottomSheet'
import { Button } from '@/components/ui/Button'
import { PageHeader, PageShell } from '@/components/ui/PageChrome'
import { MOBILE_LAYOUT_QUERY, useMediaQuery } from '@/hooks/use-media-query'
import type { ChatBackend } from '@/lib/backend'
import { sideChatQuote } from '@/lib/chat-aside'
import type { Conversation } from '@/types/chat'
import { recommendedSelection, selectedAnswers } from './answer-state'
import { DecisionSideChatPanel } from './DecisionSideChat'
import { towerFixture } from './fixtures'
import type { QuestionNeed } from './model'
import { NeedDetail } from './NeedsYou'
import { decisionSeed, selectionFromSideChat } from './side-chat'
import './tower.css'

const question = towerFixture.items[0] as QuestionNeed
const seed = decisionSeed(
  question,
  'R2 starts here. I need your decision on build isolation.',
)
const explanation =
  'An allow list still leaves the AI key and GitHub token on a machine that runs third-party build scripts. A script can read them and send them through any host on the list. Isolating keeps the keys off that machine, so a hostile build has nothing to take.'
const backend: ChatBackend = {
  id: 'fixture',
  async *stream() {
    yield { kind: 'assistant-token', token: explanation }
    yield { kind: 'assistant-end' }
  },
}
const noop = () => {}
function SideChatFixture({
  freeText = false,
  sessionGone = false,
}: {
  freeText?: boolean
  sessionGone?: boolean
}) {
  const phone = useMediaQuery(MOBILE_LAYOUT_QUERY)
  const [open, setOpen] = useState(true)
  const [kept, setKept] = useState(false)
  const [filled, setFilled] = useState(false)
  const [delivered, setDelivered] = useState(false)
  const [selections, setSelections] = useState(
    question.entry.questions.map(recommendedSelection),
  )
  const proposal = freeText
    ? 'Use a dedicated build machine with no team credentials.'
    : question.entry.questions[0].options[0].label
  const [conversation, setConversation] = useState<Conversation>({
    id: 'fixture-side-chat',
    title: 'Decision copy',
    model: 'fixture-model',
    hydrated: true,
    createdAt: 1,
    updatedAt: 1,
    status: 'done',
    sessionMetadata: {
      side_chat_of: question.sessionId,
      side_chat_question: `${question.sessionId}/${question.questionId}`,
    },
    messages: [
      {
        id: 'question-seed',
        role: 'user',
        content: `${sideChatQuote(seed.text, true)}Why isolate instead of allow-listing the network?`,
        createdAt: 2,
      },
      {
        id: 'explanation',
        role: 'assistant',
        content: explanation,
        createdAt: 3,
      },
      { id: 'proposal', role: 'assistant', content: proposal, createdAt: 4 },
    ],
  })
  const sideChat = (
    <DecisionSideChatPanel
      title="deepwork · KAN-616"
      context={182000}
      fresh={sessionGone}
      kept={kept}
      onKeep={() => setKept(true)}
      onClose={() => setOpen(false)}
      proposal={proposal}
      questions={question.entry.questions}
      onUseAnswer={
        !delivered
          ? (index, text) => {
              setSelections((current) =>
                current.map((value, i) =>
                  i === index
                    ? selectionFromSideChat(
                        question.entry.questions[index],
                        text,
                      )
                    : value,
                ),
              )
              setFilled(true)
              if (phone) setOpen(false)
            }
          : undefined
      }
    >
      <ChatView
        conversation={conversation}
        backend={backend}
        modelOptions={[{ id: 'fixture-model', label: 'Fixture model' }]}
        density="dock"
        side={{
          since: 1,
          empty: null,
          quotes: [seed],
          onJumpToQuote: noop,
          compact: phone,
        }}
        onUpdateModel={noop}
        onUpdateThinkingLevel={noop}
        onUpdateWorkingDir={noop}
        onAppendMessage={(_, message) =>
          setConversation((current) => ({
            ...current,
            messages: [...current.messages, message],
          }))
        }
        onPatchMessage={noop}
        onCompactConversation={noop}
      />
    </DecisionSideChatPanel>
  )
  return (
    <PageShell className="tower" style={{ height: '100dvh' }}>
      <PageHeader
        icon={<RadioTower />}
        title="Control tower"
        description="Needs you · a pending decision"
      />
      <div className="tower-decision-layout">
        {!phone || !open ? (
          <div className="tower-needs-detail">
            <NeedDetail
              item={question}
              entry={question.entry}
              data={towerFixture}
              record={towerFixture.questions.find(
                (row) => row.questionId === question.questionId,
              )}
              selections={selections}
              onSelection={(index, selection) =>
                setSelections((current) =>
                  current.map((value, i) => (i === index ? selection : value)),
                )
              }
              state={
                delivered
                  ? {
                      status: 'delivered',
                      summary: JSON.stringify(
                        selectedAnswers(question.entry.questions, selections),
                      ),
                    }
                  : { status: 'idle' }
              }
              canSend
              onAnswer={() => {
                setDelivered(true)
                if (!kept) setOpen(false)
              }}
              onHeld={noop}
              onResume={noop}
              onLeave={noop}
              retry={noop}
              onBack={noop}
              filledFromChat={filled}
              askAbout={
                <Button variant="ghost" size="sm" onClick={() => setOpen(true)}>
                  Ask about this <span className="tower-meta">182k</span>
                </Button>
              }
            />
          </div>
        ) : null}
        {open ? (
          phone ? (
            <BottomSheet open onOpenChange={setOpen}>
              <BottomSheetContent
                className="tower tower-decision-sheet tower-side-chat-sheet"
                heading="Side chat · pending decision"
              >
                {sideChat}
              </BottomSheetContent>
            </BottomSheet>
          ) : (
            sideChat
          )
        ) : null}
      </div>
    </PageShell>
  )
}
const meta = {
  title: 'Pages/Control tower/Side chat',
  component: SideChatFixture,
  parameters: { layout: 'fullscreen' },
} satisfies Meta<typeof SideChatFixture>
export default meta
type Story = StoryObj<typeof meta>
export const Option: Story = {}
export const FreeText: Story = { args: { freeText: true } }
export const SessionGone: Story = { args: { sessionGone: true } }
