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
import {
  MessageSquareIcon,
  MoreHorizontalIcon,
  PanelLeftCloseIcon,
  PanelLeftOpenIcon,
  PencilIcon,
  PlusIcon,
  SearchIcon,
  Trash2Icon,
} from 'lucide-react'
import { useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ConfirmDialog } from '@/components/confirm-dialog'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

import type { SessionMeta } from '../../lib/session/session-store'

type PlaygroundSessionSidebarProps = {
  sessions: SessionMeta[]
  activeId: string | null
  collapsed: boolean
  embedded?: boolean
  disabled?: boolean
  onToggle: () => void
  onCreate: () => void
  onSelect: (sessionId: string) => void
  onRename: (sessionId: string, title: string) => void
  onDelete: (sessionId: string) => void
}

type Bucket = 'today' | 'yesterday' | 'earlier'

const BUCKET_ORDER: Bucket[] = ['today', 'yesterday', 'earlier']
const BUCKET_LABEL: Record<Bucket, string> = {
  today: 'Today',
  yesterday: 'Yesterday',
  earlier: 'Earlier',
}

function bucketOf(timestamp: number): Bucket {
  const now = new Date()
  const startOfToday = new Date(
    now.getFullYear(),
    now.getMonth(),
    now.getDate()
  ).getTime()
  const startOfDay = new Date(timestamp)
  startOfDay.setHours(0, 0, 0, 0)
  const days = Math.round((startOfToday - startOfDay.getTime()) / 86400000)
  if (days <= 0) return 'today'
  if (days === 1) return 'yesterday'
  return 'earlier'
}

export function PlaygroundSessionSidebar(props: PlaygroundSessionSidebarProps) {
  const { t } = useTranslation()
  const [query, setQuery] = useState('')
  const [searchOpen, setSearchOpen] = useState(false)
  const [editingId, setEditingId] = useState<string | null>(null)
  const [editingTitle, setEditingTitle] = useState('')
  const [deleteTarget, setDeleteTarget] = useState<SessionMeta | null>(null)

  const groups = useMemo(() => {
    const keyword = query.trim().toLowerCase()
    const filtered = props.sessions.filter(
      (session) => !keyword || session.title.toLowerCase().includes(keyword)
    )
    return BUCKET_ORDER.map((bucket) => ({
      bucket,
      sessions: filtered.filter(
        (session) => bucketOf(session.updatedAt) === bucket
      ),
    })).filter((group) => group.sessions.length > 0)
  }, [props.sessions, query])

  const startEdit = (session: SessionMeta) => {
    if (props.disabled) return
    setEditingId(session.id)
    setEditingTitle(session.title || t('New chat'))
  }

  const commitEdit = () => {
    if (editingId) {
      props.onRename(editingId, editingTitle)
    }
    setEditingId(null)
  }

  const handleSelect = (sessionId: string) => {
    if (props.disabled || editingId) return
    props.onSelect(sessionId)
  }

  if (props.collapsed) {
    return (
      <div className='bg-background/95 border-border/60 absolute top-3 left-3 z-40 flex items-center gap-0.5 rounded-full border px-1.5 py-1 shadow-md backdrop-blur'>
        <div className='text-primary flex size-7 items-center justify-center'>
          <MessageSquareIcon className='size-4' />
        </div>
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                aria-label={t('New conversation')}
                className='text-muted-foreground hover:text-foreground'
                disabled={props.disabled}
                onClick={props.onCreate}
                size='icon-sm'
                variant='ghost'
              />
            }
          >
            <PlusIcon className='size-4' />
          </TooltipTrigger>
          <TooltipContent>{t('New conversation')}</TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                aria-label={t('Expand sidebar')}
                className='text-muted-foreground hover:text-foreground'
                onClick={props.onToggle}
                size='icon-sm'
                variant='ghost'
              />
            }
          >
            <PanelLeftOpenIcon className='size-4' />
          </TooltipTrigger>
          <TooltipContent>{t('Expand sidebar')}</TooltipContent>
        </Tooltip>
      </div>
    )
  }

  return (
    <aside
      className={cn(
        'border-border/60 bg-background flex h-full w-60 shrink-0 flex-col border-r',
        props.embedded && 'w-full border-r-0'
      )}
    >
      <div className='border-border/60 flex h-12 shrink-0 items-center gap-2 border-b px-3'>
        <MessageSquareIcon className='text-primary size-4 shrink-0' />
        <span className='min-w-0 flex-1 truncate text-sm font-medium'>
          {t('Conversations')}
        </span>
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                aria-label={t('Search conversations')}
                className='text-muted-foreground hover:text-foreground'
                onClick={() => setSearchOpen((open) => !open)}
                size='icon-sm'
                variant='ghost'
              />
            }
          >
            <SearchIcon className='size-4' />
          </TooltipTrigger>
          <TooltipContent>{t('Search conversations')}</TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                aria-label={t('Collapse sidebar')}
                className='text-muted-foreground hover:text-foreground'
                onClick={props.onToggle}
                size='icon-sm'
                variant='ghost'
              />
            }
          >
            <PanelLeftCloseIcon className='size-4' />
          </TooltipTrigger>
          <TooltipContent>{t('Collapse sidebar')}</TooltipContent>
        </Tooltip>
      </div>

      {searchOpen && (
        <div className='px-3 pt-2'>
          <Input
            autoFocus
            className='h-8'
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Escape') {
                setQuery('')
                setSearchOpen(false)
              }
            }}
            placeholder={t('Search conversations')}
            value={query}
          />
        </div>
      )}

      <div className='px-3 py-2'>
        <Button
          className='w-full justify-center'
          disabled={props.disabled}
          onClick={props.onCreate}
          size='sm'
        >
          <PlusIcon className='size-4' />
          {t('New conversation')}
        </Button>
      </div>

      <div className='min-h-0 flex-1 overflow-y-auto px-2 pb-3'>
        {groups.length === 0 && (
          <p className='text-muted-foreground px-2 py-6 text-center text-xs'>
            {query ? t('No matching conversations') : t('No conversations')}
          </p>
        )}
        {groups.map((group) => (
          <div key={group.bucket}>
            <div className='text-muted-foreground px-2 pt-3 pb-1 text-[10px] font-medium tracking-wider uppercase'>
              {t(BUCKET_LABEL[group.bucket])}
            </div>
            {group.sessions.map((session) => {
              const active = session.id === props.activeId
              const editing = editingId === session.id
              return (
                <div
                  aria-current={active ? 'true' : undefined}
                  aria-disabled={props.disabled || undefined}
                  className={cn(
                    'group flex items-center gap-1 rounded-md px-2 py-1.5 text-sm',
                    active ? 'bg-primary/10 text-primary' : 'hover:bg-muted/70',
                    props.disabled
                      ? 'cursor-default opacity-70'
                      : 'cursor-pointer'
                  )}
                  key={session.id}
                  onClick={() => handleSelect(session.id)}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter' || event.key === ' ') {
                      event.preventDefault()
                      handleSelect(session.id)
                    }
                  }}
                  role='button'
                  tabIndex={props.disabled ? -1 : 0}
                >
                  {editing ? (
                    <Input
                      autoFocus
                      className='h-7 px-1.5 text-sm'
                      onBlur={commitEdit}
                      onChange={(event) => setEditingTitle(event.target.value)}
                      onClick={(event) => event.stopPropagation()}
                      onKeyDown={(event) => {
                        event.stopPropagation()
                        if (event.key === 'Enter') {
                          commitEdit()
                        }
                        if (event.key === 'Escape') {
                          setEditingId(null)
                        }
                      }}
                      value={editingTitle}
                    />
                  ) : (
                    <span
                      className='min-w-0 flex-1 truncate'
                      onDoubleClick={(event) => {
                        event.stopPropagation()
                        startEdit(session)
                      }}
                    >
                      {session.title || t('New chat')}
                    </span>
                  )}
                  {!editing && (
                    <DropdownMenu>
                      <DropdownMenuTrigger
                        render={
                          <Button
                            aria-label={t('Actions')}
                            className='text-muted-foreground hover:text-foreground size-6 shrink-0 opacity-0 group-hover:opacity-100 data-[popup-open]:opacity-100'
                            disabled={props.disabled}
                            onClick={(event) => event.stopPropagation()}
                            size='icon-sm'
                            variant='ghost'
                          />
                        }
                      >
                        <MoreHorizontalIcon className='size-4' />
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align='end' className='w-36'>
                        <DropdownMenuItem
                          onClick={(event) => {
                            event.stopPropagation()
                            startEdit(session)
                          }}
                        >
                          <PencilIcon className='mr-2 size-4' />
                          {t('Rename')}
                        </DropdownMenuItem>
                        <DropdownMenuItem
                          onClick={(event) => {
                            event.stopPropagation()
                            setDeleteTarget(session)
                          }}
                          variant='destructive'
                        >
                          <Trash2Icon className='mr-2 size-4' />
                          {t('Delete')}
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  )}
                </div>
              )
            })}
          </div>
        ))}
      </div>

      <ConfirmDialog
        confirmText={t('Delete')}
        desc={t('This conversation will be permanently deleted.')}
        destructive
        handleConfirm={() => {
          if (deleteTarget) {
            props.onDelete(deleteTarget.id)
          }
          setDeleteTarget(null)
        }}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null)
        }}
        open={deleteTarget !== null}
        title={t('Delete conversation')}
      />
    </aside>
  )
}
