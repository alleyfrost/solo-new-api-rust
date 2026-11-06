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
import type { ColumnDef } from '@tanstack/react-table'
import { GitBranch, KeyRound } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { GroupBadge } from '@/components/group-badge'
import { StatusBadge, type StatusBadgeProps } from '@/components/status-badge'
import { Avatar, AvatarFallback } from '@/components/ui/avatar'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { getUserAvatarFallback, getUserAvatarStyle } from '@/lib/avatar'
import { formatTimestampToDate } from '@/lib/format'
import { cn } from '@/lib/utils'

import { LOG_TYPE_ALL_VALUE } from '../../constants'
import type { UsageLog } from '../../data/schema'
import {
  formatModelName,
  parseLogOther,
  logTokenName,
  renderAuditContent,
  renderLogContent,
} from '../../lib/format'
import {
  isDisplayableLogType,
  isTimingLogType,
  getLogTypeConfig,
} from '../../lib/utils'
import type { LogOtherData } from '../../types'
import { DetailsDialog } from '../dialogs/details-dialog'
import { ModelBadge } from '../model-badge'
import { TimingMetricsCell, StreamTpsCell } from '../timing-metrics-cell'
import { useUsageLogsContext } from '../usage-logs-provider'

interface DetailSegment {
  text: string
  muted?: boolean
  danger?: boolean
}

function buildDetailSegments(
  log: UsageLog,
  other: LogOtherData | null,
  t: (key: string, opts?: Record<string, unknown>) => string
): DetailSegment[] {
  // Top-up, audit, and login logs can carry a localized operation descriptor.
  if (log.type === 1 || log.type === 3 || log.type === 7) {
    const text = renderAuditContent(other, t) ?? renderLogContent(other, t)
    return text ? [{ text }] : []
  }

  if (log.type === 6) {
    return [{ text: t('Async task refund') }]
  }

  if (log.type !== 2 || !other) return []

  const segments: DetailSegment[] = []

  if (other.is_system_prompt_overwritten) {
    segments.push({
      text: t('System Prompt Override'),
      danger: true,
    })
  }

  return segments
}

export function useCommonLogsColumns(
  isAdmin: boolean,
  isRoot: boolean
): ColumnDef<UsageLog>[] {
  const { t } = useTranslation()
  return useMemo(() => {
    const columns: ColumnDef<UsageLog>[] = [
      {
        accessorKey: 'created_at',
        header: t('Time'),
        cell: ({ row }) => {
          const log = row.original
          const timestamp = row.getValue('created_at') as number
          const config = getLogTypeConfig(log.type)

          return (
            <div className='flex min-w-0 flex-col gap-0.5'>
              <span className='truncate font-mono text-xs tabular-nums'>
                {formatTimestampToDate(timestamp)}
              </span>
              <StatusBadge
                label={t(config.label)}
                variant={config.color as StatusBadgeProps['variant']}
                size='sm'
                copyable={false}
                className='-ml-1.5 !text-xs [&_span]:!text-xs'
              />
            </div>
          )
        },
        filterFn: (row, _id, value) => {
          if (!Array.isArray(value) || value.length === 0) return true
          if (value.includes(LOG_TYPE_ALL_VALUE)) return true
          return value.includes(String(row.original.type))
        },
        enableHiding: false,
        size: 180,
      },
    ]

    if (isAdmin) {
      columns.push(
        {
          id: 'channel',
          header: t('Channel'),
          accessorFn: (row) => row.channel,
          cell: function ChannelCell({ row }) {
            const { sensitiveVisible } = useUsageLogsContext()
            const log = row.original

            if (!isDisplayableLogType(log.type)) return null

            const other = parseLogOther(log.other)
            const rawUseChannel = other?.admin_info?.use_channel ?? []
            const useChannel = Array.isArray(rawUseChannel)
              ? rawUseChannel.map(String).filter(Boolean)
              : []
            const hasRetryChain = useChannel.length > 1
            const channelChain = hasRetryChain
              ? useChannel.join(' → ')
              : undefined
            const channelDisplay = log.channel_name
              ? `${log.channel_name} #${log.channel}`
              : `#${log.channel}`
            const channelIdDisplay = `#${log.channel}`
            const channelName = sensitiveVisible ? log.channel_name : '••••'
            const multiKeyIndex = other?.admin_info?.multi_key_index
            const showMultiKeyIndex =
              other?.admin_info?.is_multi_key === true &&
              typeof multiKeyIndex === 'number' &&
              Number.isFinite(multiKeyIndex)

            return (
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger
                    render={
                      <div className='flex max-w-[160px] flex-col gap-0.5' />
                    }
                  >
                    <div className='relative inline-flex w-fit items-center gap-1'>
                      <StatusBadge
                        label={channelIdDisplay}
                        autoColor={String(log.channel)}
                        copyText={String(log.channel)}
                        size='sm'
                        showDot={false}
                        className='font-mono'
                      />
                      {showMultiKeyIndex && (
                        <StatusBadge
                          label={String(multiKeyIndex)}
                          size='sm'
                          showDot={false}
                          copyable={false}
                          variant='neutral'
                          className='h-5 min-w-5 justify-center rounded-full px-1 font-mono text-xs'
                          aria-label={`${t('Key')} ${multiKeyIndex}`}
                        />
                      )}
                      {hasRetryChain && (
                        <Popover>
                          <PopoverTrigger
                            render={
                              <button
                                type='button'
                                className='text-muted-foreground hover:text-foreground focus-visible:ring-ring inline-flex size-5 shrink-0 items-center justify-center rounded-full transition-colors focus-visible:ring-2 focus-visible:outline-none'
                                aria-label={t('Retry Chain')}
                                onClick={(e) => e.stopPropagation()}
                              />
                            }
                          >
                            <GitBranch
                              className='size-3.5 text-amber-500'
                              aria-hidden='true'
                            />
                          </PopoverTrigger>
                          <PopoverContent
                            side='top'
                            align='start'
                            className='w-64 text-xs'
                          >
                            <div className='flex flex-col gap-1'>
                              <p className='font-medium'>{t('Retry Chain')}</p>
                              <p className='text-muted-foreground font-mono break-all'>
                                {channelChain}
                              </p>
                            </div>
                          </PopoverContent>
                        </Popover>
                      )}
                    </div>
                    {log.channel_name && (
                      <span className='text-muted-foreground/70 truncate [font-family:var(--font-body)] !text-xs'>
                        {channelName}
                      </span>
                    )}
                  </TooltipTrigger>
                  <TooltipContent>
                    <div className='space-y-1'>
                      <p>
                        {sensitiveVisible ? channelDisplay : channelIdDisplay}
                      </p>
                      {channelChain && (
                        <p className='text-muted-foreground text-xs'>
                          {t('Chain')}: {channelChain}
                        </p>
                      )}
                      {showMultiKeyIndex && (
                        <p className='text-muted-foreground text-xs'>
                          {t('Key')}: {multiKeyIndex}
                        </p>
                      )}
                    </div>
                  </TooltipContent>
                </Tooltip>
              </TooltipProvider>
            )
          },
        },
        {
          id: 'user',
          header: t('User'),
          accessorFn: (row) => row.username,
          cell: function UserCell({ row }) {
            const {
              sensitiveVisible,
              setSelectedUserId,
              setUserInfoDialogOpen,
            } = useUsageLogsContext()
            const log = row.original

            if (!log.username) return null

            return (
              <button
                type='button'
                className='flex items-center gap-1.5 text-left'
                onClick={(e) => {
                  e.stopPropagation()
                  setSelectedUserId(log.user_id)
                  setUserInfoDialogOpen(true)
                }}
              >
                <Avatar className='ring-border/60 size-6 ring-1 max-sm:hidden'>
                  <AvatarFallback
                    className={cn(
                      'text-[11px] font-semibold',
                      !sensitiveVisible && 'bg-muted text-muted-foreground'
                    )}
                    style={
                      sensitiveVisible
                        ? getUserAvatarStyle(log.username)
                        : undefined
                    }
                  >
                    {sensitiveVisible
                      ? getUserAvatarFallback(log.username)
                      : '•'}
                  </AvatarFallback>
                </Avatar>
                <TooltipProvider delay={300}>
                  <Tooltip>
                    <TooltipTrigger
                      render={
                        <span className='text-muted-foreground max-w-[100px] truncate text-sm hover:underline' />
                      }
                    >
                      {sensitiveVisible ? log.username : '••••'}
                    </TooltipTrigger>
                    {sensitiveVisible && log.username.length > 12 && (
                      <TooltipContent side='top'>{log.username}</TooltipContent>
                    )}
                  </Tooltip>
                </TooltipProvider>
              </button>
            )
          },
        }
      )
    }

    columns.push({
      accessorKey: 'token_name',
      header: t('Token'),
      cell: function TokenNameCell({ row }) {
        const { sensitiveVisible } = useUsageLogsContext()
        const log = row.original
        if (!isDisplayableLogType(log.type)) return null

        const tokenName = logTokenName(log, t)
        if (!tokenName) return null

        const other = parseLogOther(log.other)
        const displayName = sensitiveVisible ? tokenName : '••••'
        let group = log.group
        if (!group) group = other?.group || ''

        return (
          <div className='flex max-w-[200px] flex-col gap-0.5'>
            <TooltipProvider delay={300}>
              <Tooltip>
                <TooltipTrigger render={<div className='max-w-full' />}>
                  <StatusBadge
                    label={displayName}
                    icon={KeyRound}
                    copyText={sensitiveVisible ? log.token_name : undefined}
                    size='sm'
                    showDot={false}
                    className='border-border/60 bg-muted/30 text-foreground h-6 max-w-full gap-1.5 overflow-hidden rounded-md border px-2 py-0.5 [font-family:var(--font-body)]'
                  />
                </TooltipTrigger>
                {sensitiveVisible && tokenName.length > 16 && (
                  <TooltipContent side='top' className='max-w-xs break-all'>
                    {tokenName}
                  </TooltipContent>
                )}
              </Tooltip>
            </TooltipProvider>
            {group ? (
              <span className='block max-w-full truncate text-xs leading-none'>
                <GroupBadge
                  group={group}
                  label={sensitiveVisible ? undefined : '••••'}
                  type='text'
                  size='sm'
                  className='inline align-baseline text-xs leading-none [&>span]:leading-none'
                />
              </span>
            ) : null}
          </div>
        )
      },
      size: 160,
    })
    columns.push(
      {
        accessorKey: 'model_name',
        header: t('Model'),
        cell: function ModelCell({ row }) {
          const log = row.original
          if (!isDisplayableLogType(log.type)) return null

          const modelInfo = formatModelName(log)

          return (
            <div className='flex w-fit flex-col gap-0.5'>
              <ModelBadge
                modelName={modelInfo.name}
                actualModel={modelInfo.actualModel}
                responseModel={modelInfo.responseModel}
              />
            </div>
          )
        },
        meta: { mobileTitle: true },
      },
      {
        accessorKey: 'is_stream',
        header: t('Stream'),
        cell: ({ row }) => {
          const log = row.original
          if (!isTimingLogType(log.type)) return null

          const useTime = row.getValue('use_time') as number
          const other = parseLogOther(log.other)
          const tokensPerSecond =
            useTime > 0 && log.completion_tokens > 0
              ? log.completion_tokens / useTime
              : null

          return (
            <StreamTpsCell
              isStream={log.is_stream}
              isTask={other?.is_task === true}
              isSyncTask={other?.task_sync === true}
              tokensPerSecond={tokensPerSecond}
              streamStatus={other?.stream_status}
            />
          )
        },
        meta: { label: t('Stream') },
      },
      {
        accessorKey: 'prompt_tokens',
        header: t('Tokens', { context: 'usage' }),
        cell: ({ row }) => {
          const log = row.original
          if (!isDisplayableLogType(log.type)) return null

          const other = parseLogOther(log.other)

          const promptTokens = log.prompt_tokens || 0
          const completionTokens = log.completion_tokens || 0
          if (promptTokens === 0 && completionTokens === 0) {
            return <span className='text-muted-foreground text-xs'>-</span>
          }

          const cacheReadTokens = other?.cache_tokens || 0
          const cacheWrite5m = other?.cache_creation_tokens_5m || 0
          const cacheWrite1h = other?.cache_creation_tokens_1h || 0
          const hasSplitCache = cacheWrite5m > 0 || cacheWrite1h > 0
          const cacheWriteTokens = hasSplitCache
            ? cacheWrite5m + cacheWrite1h
            : other?.cache_creation_tokens || 0

          return (
            <div className='flex flex-col gap-0.5'>
              <span className='font-mono text-xs font-medium tabular-nums'>
                {promptTokens.toLocaleString()} /{' '}
                {completionTokens.toLocaleString()}
              </span>
              {(cacheReadTokens > 0 || cacheWriteTokens > 0) && (
                <div className='flex items-center gap-1 text-[11px]'>
                  {cacheReadTokens > 0 && (
                    <span className='text-muted-foreground/60'>
                      {t('Cache')}↓ {cacheReadTokens.toLocaleString()}
                    </span>
                  )}
                  {cacheWriteTokens > 0 && (
                    <span className='text-muted-foreground/60'>
                      ↑ {cacheWriteTokens.toLocaleString()}
                    </span>
                  )}
                </div>
              )}
            </div>
          )
        },
      },
      {
        id: 'total_tokens',
        header: t('Total Tokens'),
        accessorFn: (row) =>
          (row.prompt_tokens || 0) + (row.completion_tokens || 0),
        cell: ({ row }) => {
          const log = row.original
          if (!isDisplayableLogType(log.type)) return null

          const totalTokens = row.getValue('total_tokens') as number
          if (totalTokens <= 0) {
            return <span className='text-muted-foreground text-xs'>-</span>
          }

          return (
            <span className='font-mono text-xs font-medium tabular-nums'>
              {totalTokens.toLocaleString()}
            </span>
          )
        },
      },
      {
        accessorKey: 'use_time',
        header: t('Timing'),
        cell: ({ row }) => {
          const log = row.original
          if (!isTimingLogType(log.type)) return null

          const useTime = row.getValue('use_time') as number
          const other = parseLogOther(log.other)

          return (
            <TimingMetricsCell
              useTimeSec={useTime}
              completionTokens={log.completion_tokens}
              frtMs={other?.frt}
              isStream={log.is_stream}
            />
          )
        },
      },

      {
        accessorKey: 'content',
        header: t('Details'),
        cell: function DetailsCell({ row }) {
          const { t } = useTranslation()
          const [dialogOpen, setDialogOpen] = useState(false)
          const log = row.original
          const other = parseLogOther(log.other)

          const segments = buildDetailSegments(log, other, t)
          const primary = segments[0]
          const hasMore = segments.length > 1
          let primaryTextClass = 'text-foreground'
          if (primary?.muted) {
            primaryTextClass = 'text-muted-foreground/60'
          } else if (primary?.danger) {
            primaryTextClass = 'text-red-600 dark:text-red-400'
          }
          let detailPreview = (
            <span className='text-muted-foreground/40'>—</span>
          )
          if (primary) {
            detailPreview = (
              <span
                className={cn(
                  'truncate leading-snug group-hover:underline',
                  primaryTextClass
                )}
              >
                {primary.text}
                {hasMore && (
                  <span className='text-muted-foreground/40 ml-0.5'>
                    +{segments.length - 1}
                  </span>
                )}
              </span>
            )
          } else if (log.content) {
            detailPreview = (
              <span className='text-muted-foreground truncate group-hover:underline'>
                {renderLogContent(other, t) ?? log.content}
              </span>
            )
          }

          return (
            <>
              <button
                type='button'
                className='group flex max-w-[200px] items-center gap-1 text-left text-xs'
                onClick={() => setDialogOpen(true)}
                title={t('Click to view full details')}
              >
                {detailPreview}
              </button>
              <DetailsDialog
                log={log}
                isAdmin={isAdmin}
                isRoot={isRoot}
                open={dialogOpen}
                onOpenChange={setDialogOpen}
              />
            </>
          )
        },
        size: 180,
        maxSize: 200,
      }
    )

    return columns
  }, [t, isAdmin, isRoot])
}
