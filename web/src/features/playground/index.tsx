/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Sheet, SheetContent, SheetTitle } from '@/components/ui/sheet'
import { useMediaQuery } from '@/hooks'

import { PlaygroundChat } from './components/chat/playground-chat'
import { PlaygroundInput } from './components/input/playground-input'
import { PlaygroundSessionSidebar } from './components/session/playground-session-sidebar'
import {
  useChatHandler,
  usePlaygroundConversation,
  usePlaygroundOptions,
  usePlaygroundState,
} from './hooks'

export function Playground() {
  const { t } = useTranslation()
  const isMobile = useMediaQuery('(max-width: 767px)')
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false)
  const [drawerOpen, setDrawerOpen] = useState(false)

  const {
    config,
    parameterEnabled,
    messages,
    isLoadingMessages,
    models,
    groups,
    sessions,
    activeId,
    draft,
    updateMessages,
    updateSessionMessages,
    setModels,
    setGroups,
    updateConfig,
    updateParameterEnabled,
    updateDraft,
    selectSession,
    createSession,
    renameSession,
    deleteSession,
    clearMessages,
  } = usePlaygroundState()

  const {
    sendChat,
    stopGeneration,
    cancelSessionGeneration,
    runningSessionIds,
    isGenerating,
  } = useChatHandler({
    activeSessionId: activeId,
    config,
    parameterEnabled,
    onMessageUpdate: updateSessionMessages,
  })

  const {
    editingMessageKey,
    handleSendMessage,
    handleRegenerateMessage,
    handleEditMessage,
    handleEditOpenChange,
    applyEdit,
    handleDeleteMessage,
  } = usePlaygroundConversation({
    messages,
    updateMessages,
    sendChat,
  })

  const handleClearMessages = () => {
    handleEditOpenChange(false)
    clearMessages()
  }

  const handleCreateSession = () => {
    if (!createSession()) {
      toast.error(t('Conversation limit reached'), {
        description: t('Delete an old conversation first.'),
      })
      return
    }
    if (isMobile) setDrawerOpen(false)
  }

  const handleSelectSession = (sessionId: string) => {
    selectSession(sessionId)
    if (isMobile) setDrawerOpen(false)
  }

  const handleDeleteSession = (sessionId: string) => {
    cancelSessionGeneration(sessionId)
    void deleteSession(sessionId)
  }

  const { isLoadingModels } = usePlaygroundOptions({
    currentGroup: config.group,
    currentModel: config.model,
    setGroups,
    setModels,
    updateConfig,
  })

  const sidebar = (
    collapsed: boolean,
    options: { embedded?: boolean; onToggle?: () => void } = {}
  ) => (
    <PlaygroundSessionSidebar
      activeId={activeId}
      collapsed={collapsed}
      embedded={options.embedded}
      runningSessionIds={runningSessionIds}
      onCreate={handleCreateSession}
      onDelete={handleDeleteSession}
      onRename={renameSession}
      onSelect={handleSelectSession}
      onToggle={
        options.onToggle ?? (() => setSidebarCollapsed((value) => !value))
      }
      sessions={sessions}
    />
  )

  return (
    <div className='relative flex size-full min-h-0 overflow-hidden'>
      {!isMobile && !sidebarCollapsed && sidebar(false)}

      <div className='relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden'>
        {!isMobile && sidebarCollapsed && sidebar(true)}

        {isMobile && (
          <>
            <Sheet open={drawerOpen} onOpenChange={setDrawerOpen}>
              <SheetContent className='w-72 p-0' side='left'>
                <SheetTitle className='sr-only'>
                  {t('Conversations')}
                </SheetTitle>
                {sidebar(false, {
                  embedded: true,
                  onToggle: () => setDrawerOpen(false),
                })}
              </SheetContent>
            </Sheet>
            {!drawerOpen &&
              sidebar(true, { onToggle: () => setDrawerOpen(true) })}
          </>
        )}

        {/* Full-width scroll container: scrolling works even over side whitespace */}
        <div className='flex min-h-0 flex-1 flex-col overflow-hidden'>
          <PlaygroundChat
            messages={messages}
            isLoadingMessages={isLoadingMessages}
            onRegenerateMessage={handleRegenerateMessage}
            onEditMessage={handleEditMessage}
            onDeleteMessage={handleDeleteMessage}
            onSelectPrompt={handleSendMessage}
            isGenerating={isGenerating}
            editingKey={editingMessageKey}
            onCancelEdit={handleEditOpenChange}
            onSaveEdit={(newContent) => applyEdit(newContent, false)}
            onSaveEditAndSubmit={(newContent) => applyEdit(newContent, true)}
          />
        </div>

        {/* Input area: center content and constrain to the same container width */}
        <div className='mx-auto w-full max-w-4xl'>
          <PlaygroundInput
            config={config}
            disabled={isGenerating}
            draft={draft}
            groups={groups}
            groupValue={config.group}
            isGenerating={isGenerating}
            isModelLoading={isLoadingModels}
            modelValue={config.model}
            models={models}
            onConfigChange={updateConfig}
            onClearMessages={handleClearMessages}
            onDraftChange={updateDraft}
            onGroupChange={(value) => updateConfig('group', value)}
            onModelChange={(value) => updateConfig('model', value)}
            onParameterEnabledChange={updateParameterEnabled}
            onStop={() => stopGeneration()}
            onSubmit={handleSendMessage}
            parameterEnabled={parameterEnabled}
            hasMessages={messages.length > 0}
          />
        </div>
      </div>
    </div>
  )
}
