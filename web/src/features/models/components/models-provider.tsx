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
/* eslint-disable react-refresh/only-export-components */
import React, { createContext, useContext, useState } from 'react'

import type { Model } from '../types'

// ============================================================================
// Types
// ============================================================================

type DialogType = 'create-model' | 'update-model' | 'description' | null

type ModelsContextType = {
  open: DialogType
  setOpen: (open: DialogType) => void
  currentRow: Model | null
  setCurrentRow: (model: Model | null) => void
  descriptionData: { modelName: string; description: string } | null
  setDescriptionData: (
    data: { modelName: string; description: string } | null
  ) => void
}

// ============================================================================
// Context
// ============================================================================

const ModelsContext = createContext<ModelsContextType | undefined>(undefined)

// ============================================================================
// Provider
// ============================================================================

export function ModelsProvider({ children }: { children: React.ReactNode }) {
  const [open, setOpen] = useState<DialogType>(null)
  const [currentRow, setCurrentRow] = useState<Model | null>(null)
  const [descriptionData, setDescriptionData] = useState<{
    modelName: string
    description: string
  } | null>(null)

  return (
    <ModelsContext.Provider
      value={{
        open,
        setOpen,
        currentRow,
        setCurrentRow,
        descriptionData,
        setDescriptionData,
      }}
    >
      {children}
    </ModelsContext.Provider>
  )
}

// ============================================================================
// Hook
// ============================================================================

export function useModels() {
  const context = useContext(ModelsContext)
  if (!context) {
    throw new Error('useModels must be used within ModelsProvider')
  }
  return context
}
