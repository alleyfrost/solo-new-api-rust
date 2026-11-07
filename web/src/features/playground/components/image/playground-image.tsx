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
  DownloadIcon,
  ImageIcon,
  Loader2Icon,
  WandSparklesIcon,
} from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ModelGroupSelector } from '@/components/model-group-selector'
import { Button } from '@/components/ui/button'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Textarea } from '@/components/ui/textarea'
import { getServerErrorMessage } from '@/lib/server-error-message'

import { generateImage } from '../../api'
import type { GroupOption, ModelOption } from '../../types'

const IMAGE_SIZES = ['2048x2048', '4096x4096'] as const
const DEFAULT_SIZE = IMAGE_SIZES[0]

interface GeneratedImage {
  src: string
  prompt: string
  totalTokens?: number
}

interface PlaygroundImageProps {
  model: string
  group: string
  models: ModelOption[]
  groups: GroupOption[]
  isModelLoading?: boolean
  onModelChange: (value: string) => void
  onGroupChange: (value: string) => void
}

function imageSource(data: { b64_json?: string; url?: string } | undefined) {
  if (!data) return null
  if (data.b64_json) return `data:image/png;base64,${data.b64_json}`
  return data.url ?? null
}

/** 生图模型不走对话：这里是一个独立的出图面板，不写入聊天记录与本地存储。 */
export function PlaygroundImage({
  model,
  group,
  models,
  groups,
  isModelLoading = false,
  onModelChange,
  onGroupChange,
}: PlaygroundImageProps) {
  const { t } = useTranslation()
  const [prompt, setPrompt] = useState('')
  const [size, setSize] = useState<string>(DEFAULT_SIZE)
  const [busy, setBusy] = useState(false)
  const [image, setImage] = useState<GeneratedImage | null>(null)
  const [error, setError] = useState<string | null>(null)

  const submit = async () => {
    const text = prompt.trim()
    if (!text || busy) return
    setBusy(true)
    setError(null)
    try {
      const response = await generateImage({ model, group, prompt: text, size })
      const src = imageSource(response?.data?.[0])
      if (!src) throw new Error(t('Image generation failed'))
      const tokens = response?.usage?.total_tokens
      setImage({
        src,
        prompt: text,
        totalTokens: typeof tokens === 'number' ? tokens : undefined,
      })
    } catch (caught) {
      setError(getServerErrorMessage(caught, t('Image generation failed')))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className='mx-auto flex w-full max-w-4xl flex-1 flex-col gap-4 overflow-y-auto p-4 sm:p-6'>
      <div className='flex flex-wrap items-center justify-between gap-3'>
        <div className='flex items-center gap-2 text-sm font-medium'>
          <ImageIcon className='text-primary h-4 w-4' />
          {t('Image Generation')}
        </div>
        <div className='flex items-center gap-2'>
          <ModelGroupSelector
            selectedModel={model}
            models={models}
            onModelChange={onModelChange}
            selectedGroup={group}
            groups={groups}
            onGroupChange={onGroupChange}
            disabled={busy || isModelLoading}
          />
          <Select
            value={size}
            onValueChange={(value) => setSize(value ?? DEFAULT_SIZE)}
            disabled={busy}
          >
            <SelectTrigger
              className='h-8 w-32 text-xs'
              aria-label={t('Image size')}
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {IMAGE_SIZES.map((option) => (
                <SelectItem key={option} value={option} className='text-xs'>
                  {option}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>

      <div className='bg-card flex min-h-64 flex-1 items-center justify-center rounded-xl border p-4'>
        {busy ? (
          <div className='text-muted-foreground flex items-center gap-2 text-sm'>
            <Loader2Icon className='h-4 w-4 animate-spin' />
            {t('Generating...')}
          </div>
        ) : image ? (
          <figure className='flex max-h-[60vh] flex-col items-center gap-3'>
            <img
              src={image.src}
              alt={image.prompt}
              className='max-h-[52vh] w-auto rounded-lg object-contain shadow-sm'
            />
            <figcaption className='text-muted-foreground flex items-center gap-3 text-xs'>
              <span className='max-w-md truncate'>{image.prompt}</span>
              {image.totalTokens !== undefined && (
                <span>
                  {t('Tokens')}: {image.totalTokens}
                </span>
              )}
              <a
                className='text-primary inline-flex items-center gap-1 hover:underline'
                href={image.src}
                download={`image-${Date.now()}.png`}
              >
                <DownloadIcon className='h-3.5 w-3.5' />
                {t('Download')}
              </a>
            </figcaption>
          </figure>
        ) : (
          <p className='text-muted-foreground text-sm'>
            {t('Generated images will appear here.')}
          </p>
        )}
      </div>

      <div className='space-y-2'>
        <Textarea
          value={prompt}
          rows={3}
          placeholder={t('Describe the image you want')}
          onChange={(event) => setPrompt(event.target.value)}
          onKeyDown={(event) => {
            if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
              void submit()
            }
          }}
          disabled={busy}
        />
        <div className='flex items-center justify-between gap-3'>
          <p className='text-muted-foreground text-xs'>
            {error ? (
              <span className='text-destructive'>{error}</span>
            ) : (
              t('Press ⌘/Ctrl + Enter to generate')
            )}
          </p>
          <Button
            type='button'
            onClick={() => void submit()}
            disabled={busy || !prompt.trim()}
          >
            {busy ? (
              <Loader2Icon className='mr-1.5 h-4 w-4 animate-spin' />
            ) : (
              <WandSparklesIcon className='mr-1.5 h-4 w-4' />
            )}
            {t('Generate Image')}
          </Button>
        </div>
      </div>
    </div>
  )
}
