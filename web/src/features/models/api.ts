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
import { api } from '@/lib/api'

import type {
  GetModelsParams,
  GetModelsResponse,
  GetModelResponse,
  GetVendorsResponse,
  Model,
  SearchModelsParams,
} from './types'

// ============================================================================
// Model CRUD Operations
// ============================================================================

/**
 * Get paginated list of models
 */
export async function getModels(
  params: GetModelsParams = {}
): Promise<GetModelsResponse> {
  const res = await api.get('/api/models/', { params })
  return res.data
}

/**
 * Search models with filters
 */
export async function searchModels(
  params: SearchModelsParams
): Promise<GetModelsResponse> {
  const res = await api.get('/api/models/search', { params })
  return res.data
}

/**
 * Get single model by ID
 */
export async function getModel(id: number): Promise<GetModelResponse> {
  const res = await api.get(`/api/models/${id}`)
  return res.data
}

/**
 * Create new model
 */
export async function createModel(
  data: Partial<Model>
): Promise<{ success: boolean; message?: string; data?: Model }> {
  const res = await api.post('/api/models/', data, {
    skipBusinessError: true,
    skipErrorHandler: true,
  })
  return res.data
}

/**
 * Update existing model
 */
export async function updateModel(
  data: Partial<Model> & { id: number }
): Promise<{ success: boolean; message?: string; data?: Model }> {
  const res = await api.put('/api/models/', data, {
    skipBusinessError: true,
    skipErrorHandler: true,
  })
  return res.data
}

export interface ModelDeleteResult {
  deleted?: number
}

/**
 * Delete a single model (the server soft deletes it)
 */
export async function deleteModel(
  id: number
): Promise<{ success: boolean; message?: string; data?: ModelDeleteResult }> {
  const res = await api.delete(`/api/models/${id}`)
  return res.data
}

/**
 * Delete several models. The server only exposes single model deletion, so a
 * batch runs one request per model and reports the first failure verbatim.
 */
export async function deleteModels(
  modelIds: number[]
): Promise<{ success: boolean; message?: string; data: ModelDeleteResult }> {
  const results = await Promise.all(modelIds.map((id) => deleteModel(id)))
  const failed = results.find((result) => !result.success)
  if (failed) {
    return { success: false, message: failed.message, data: { deleted: 0 } }
  }
  return { success: true, data: { deleted: modelIds.length } }
}

// ============================================================================
// Vendor Reference Data (read only)
// ============================================================================

/**
 * Get the vendor list used by model metadata selection and filtering
 */
export async function getVendors(params?: {
  p?: number
  page_size?: number
}): Promise<GetVendorsResponse> {
  const res = await api.get('/api/vendors/', {
    params: params || { page_size: 1000 },
  })
  return res.data
}
