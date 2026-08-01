<template>
  <el-card shadow="never" class="top-list-card">
    <template #header>
      <div class="card-header">
        <el-icon class="card-icon"><Connection /></el-icon>
        <span>活跃客户端 (Top 10)</span>
      </div>
    </template>

    <div v-loading="loading" class="list-container">
      <div v-if="stats.length === 0" class="empty-state">
        <el-empty description="暂无数据" :image-size="60" />
      </div>

      <div v-else class="rank-list">
        <div v-for="(item, index) in stats" :key="item.name" class="rank-item">
          <div class="rank-index" :class="'rank-' + (index + 1)">{{ index + 1 }}</div>
          <div class="rank-content">
            <div class="item-name">{{ item.name }}</div>
            <div class="item-bar">
              <div class="bar-fill" :style="{ width: getPercentage(item.count) + '%' }"></div>
            </div>
          </div>
          <div class="item-count">{{ item.count }}次</div>
        </div>
      </div>
    </div>
  </el-card>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { Connection } from '@element-plus/icons-vue'
import api from '../../api'

interface TopStats {
  name: string
  count: number
}

const stats = ref<TopStats[]>([])
const loading = ref(false)
let refreshInterval: ReturnType<typeof setInterval> | null = null

function getPercentage(count: number): number {
  if (stats.value.length === 0) return 0
  const max = stats.value[0]?.count || 1
  return Math.min(100, (count / max) * 100)
}

async function fetchData() {
  try {
    if (stats.value.length === 0) loading.value = true

    const response = await api.get<TopStats[]>('/api/stats/top/clients')
    stats.value = response.data
  } catch (error) {
    console.error('Failed to fetch top clients:', error)
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  fetchData()
  refreshInterval = setInterval(fetchData, 10000) // Refresh every 10s
})

onUnmounted(() => {
  if (refreshInterval) clearInterval(refreshInterval)
})
</script>

<style scoped>
.top-list-card {
  height: 100%;
  border-radius: 12px;
  border: none;
}

.card-header {
  display: flex;
  align-items: center;
  gap: 8px;
  font-weight: 600;
  color: #303133;
}

.card-icon {
  font-size: 18px;
  color: #67c23a;  /* Different color for clients */
}

.list-container {
  min-height: 300px;
}

.rank-list {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.rank-item {
  display: flex;
  align-items: center;
  gap: 12px;
}

.rank-index {
  width: 24px;
  height: 24px;
  border-radius: 6px;
  background: #f0f2f5;
  color: #909399;
  font-size: 12px;
  font-weight: 700;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.rank-1 {
  background: #ffe1e1;
  color: #f56c6c;
}

.rank-2 {
  background: #fff3e0;
  color: #e6a23c;
}

.rank-3 {
  background: #e1f3d8;
  color: #67c23a;
}

.rank-content {
  flex: 1;
  min-width: 0;
}

.item-name {
  font-size: 14px;
  color: #606266;
  margin-bottom: 6px;
}

.item-bar {
  height: 6px;
  background: #f0f2f5;
  border-radius: 3px;
  overflow: hidden;
}

.bar-fill {
  height: 100%;
  background: #67c23a; /* Green gradient */
  border-radius: 3px;
}

.item-count {
  font-size: 13px;
  color: #909399;
  width: 60px;
  text-align: right;
  flex-shrink: 0;
}
</style>
