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
import type { Message } from '../../types'
import {
  extractInlineImageUrls,
  OMITTED_IMAGE_MARKER,
} from '../message/message-images'

const DB_NAME = 'solo-playground'
const STORE_NAME = 'session_images'
const DB_VERSION = 1
/** Keep images for the last few image-bearing messages, older records are evicted. */
export const MAX_CACHED_IMAGE_MESSAGES = 4
/** A single reference image larger than this is not worth persisting. */
export const MAX_CACHED_IMAGE_BYTES = 12 * 1024 * 1024

type CachedImage = {
  type: string
  data: ArrayBuffer
}

type ImageRecord = {
  id: string
  images: CachedImage[]
}

let dbPromise: Promise<IDBDatabase> | null = null
const signatures = new Map<string, string>()

function openDb(): Promise<IDBDatabase> {
  if (typeof indexedDB === 'undefined') {
    return Promise.reject(new Error('IndexedDB is not available'))
  }
  if (!dbPromise) {
    const opening = new Promise<IDBDatabase>((resolve, reject) => {
      const request = indexedDB.open(DB_NAME, DB_VERSION)
      request.onupgradeneeded = () => {
        const db = request.result
        if (!db.objectStoreNames.contains(STORE_NAME)) {
          db.createObjectStore(STORE_NAME, { keyPath: 'id' })
        }
      }
      request.addEventListener('success', () => resolve(request.result))
      request.addEventListener('error', () => reject(request.error))
    })
    dbPromise = opening
    opening.catch(() => {
      if (dbPromise === opening) {
        dbPromise = null
      }
    })
  }
  return dbPromise
}

function requestToPromise<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.addEventListener('success', () => resolve(request.result))
    request.addEventListener('error', () => reject(request.error))
  })
}

function recordId(
  userId: number,
  sessionId: string,
  messageKey: string,
  versionId: string
): string {
  return `${userId}:${sessionId}:${messageKey}:${versionId}`
}

function dataUrlsOf(content: string): string[] {
  return extractInlineImageUrls(content).filter((url) =>
    url.startsWith('data:image/')
  )
}

function hashUrls(urls: string[]): string {
  let hash = 0x811c9dc5
  for (const url of urls) {
    for (let index = 0; index < url.length; index++) {
      hash ^= url.charCodeAt(index)
      hash = Math.imul(hash, 0x01000193)
    }
  }
  const total = urls.reduce((size, url) => size + url.length, 0)
  return `${urls.length}:${total}:${(hash >>> 0).toString(36)}`
}

function dataUrlToImage(dataUrl: string): CachedImage | null {
  const comma = dataUrl.indexOf(',')
  if (comma === -1) {
    return null
  }
  const header = dataUrl.slice(5, comma)
  if (!header.includes(';base64')) {
    return null
  }
  const type = header.split(';')[0] || 'image/png'
  try {
    const binary = atob(dataUrl.slice(comma + 1))
    const bytes = new Uint8Array(binary.length)
    for (let index = 0; index < binary.length; index++) {
      bytes[index] = binary.charCodeAt(index)
    }
    return { type, data: bytes.buffer }
  } catch {
    return null
  }
}

function cachedImageToDataUrl(image: CachedImage): string {
  const bytes = new Uint8Array(image.data)
  let binary = ''
  const chunk = 0x8000
  for (let offset = 0; offset < bytes.length; offset += chunk) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + chunk))
  }
  return `data:${image.type || 'image/png'};base64,${btoa(binary)}`
}

function fillMarkers(content: string, dataUrls: string[]): string {
  if (dataUrls.length === 0 || !content.includes(OMITTED_IMAGE_MARKER)) {
    return content
  }
  let cursor = 0
  return content.replaceAll(OMITTED_IMAGE_MARKER, () => {
    const url = dataUrls[cursor]
    if (!url) {
      return OMITTED_IMAGE_MARKER
    }
    cursor += 1
    return `![image](${url})`
  })
}

export async function cacheSessionImages(
  userId: number,
  sessionId: string,
  messages: Message[]
): Promise<void> {
  const db = await openDb()
  const prefix = `${userId}:${sessionId}:`
  const imageMessages = messages.filter((message) =>
    message.versions.some((version) => dataUrlsOf(version.content).length > 0)
  )
  const recent = imageMessages.slice(-MAX_CACHED_IMAGE_MESSAGES)
  const keepIds = new Set<string>()

  const put = (record: ImageRecord) =>
    requestToPromise(
      db
        .transaction(STORE_NAME, 'readwrite')
        .objectStore(STORE_NAME)
        .put(record)
    ).then(() => undefined)

  for (const message of recent) {
    for (const version of message.versions) {
      const urls = dataUrlsOf(version.content)
      if (urls.length === 0) {
        continue
      }
      const id = recordId(userId, sessionId, message.key, version.id)
      keepIds.add(id)
      const signature = hashUrls(urls)
      if (signatures.get(id) === signature) {
        continue
      }
      const images: CachedImage[] = []
      for (const url of urls) {
        if (url.length > (MAX_CACHED_IMAGE_BYTES * 4) / 3) {
          continue
        }
        const image = dataUrlToImage(url)
        if (image) {
          images.push(image)
        }
      }
      if (images.length === 0) {
        continue
      }
      await put({ id, images })
      signatures.set(id, signature)
    }
  }

  const keys = await requestToPromise(
    db.transaction(STORE_NAME, 'readonly').objectStore(STORE_NAME).getAllKeys()
  )
  const remove = (key: IDBValidKey) =>
    requestToPromise(
      db
        .transaction(STORE_NAME, 'readwrite')
        .objectStore(STORE_NAME)
        .delete(key)
    ).then(() => undefined)

  for (const key of keys) {
    if (
      typeof key !== 'string' ||
      !key.startsWith(prefix) ||
      keepIds.has(key)
    ) {
      continue
    }
    await remove(key)
    signatures.delete(key)
  }
}

export async function rehydrateSessionImages(
  userId: number,
  sessionId: string,
  messages: Message[]
): Promise<Message[]> {
  const db = await openDb()
  const result: Message[] = []

  for (const message of messages) {
    let changed = false
    const versions: Message['versions'] = []
    for (const version of message.versions) {
      if (!version.content.includes(OMITTED_IMAGE_MARKER)) {
        versions.push(version)
        continue
      }
      const record = (await requestToPromise(
        db
          .transaction(STORE_NAME, 'readonly')
          .objectStore(STORE_NAME)
          .get(recordId(userId, sessionId, message.key, version.id))
      )) as ImageRecord | undefined
      if (!record?.images?.length) {
        versions.push(version)
        continue
      }
      const dataUrls = record.images.map(cachedImageToDataUrl)
      const content = fillMarkers(version.content, dataUrls)
      versions.push(
        content === version.content ? version : { ...version, content }
      )
      changed ||= content !== version.content
    }
    result.push(changed ? { ...message, versions } : message)
  }

  return result
}

export async function deleteSessionImages(
  userId: number,
  sessionId: string
): Promise<void> {
  const db = await openDb()
  const prefix = `${userId}:${sessionId}:`
  const keys = await requestToPromise(
    db.transaction(STORE_NAME, 'readonly').objectStore(STORE_NAME).getAllKeys()
  )
  const store = db.transaction(STORE_NAME, 'readwrite').objectStore(STORE_NAME)
  for (const key of keys) {
    if (typeof key === 'string' && key.startsWith(prefix)) {
      store.delete(key)
      signatures.delete(key)
    }
  }
}

export async function deleteUserImages(userId: number): Promise<void> {
  const db = await openDb()
  const prefix = `${userId}:`
  const keys = await requestToPromise(
    db.transaction(STORE_NAME, 'readonly').objectStore(STORE_NAME).getAllKeys()
  )
  const store = db.transaction(STORE_NAME, 'readwrite').objectStore(STORE_NAME)
  for (const key of keys) {
    if (typeof key === 'string' && key.startsWith(prefix)) {
      store.delete(key)
      signatures.delete(key)
    }
  }
}
