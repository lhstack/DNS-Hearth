<template>
  <div class="dns-records">
    <!-- 页面标题 -->
    <div class="page-header">
      <div class="header-left">
        <h1>DNS 记录管理</h1>
        <p class="subtitle">管理本地 DNS 记录，支持 A、AAAA、CNAME、MX 等类型</p>
      </div>
      <el-button type="primary" size="large" @click="openCreateDialog">
        <el-icon><Plus /></el-icon>
        添加记录
      </el-button>
    </div>

    <!-- 统计卡片 -->
    <el-row :gutter="20" class="stats-row">
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);">
            <el-icon><Document /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ records.length }}</span>
            <span class="stat-label">总记录数</span>
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: linear-gradient(135deg, #11998e 0%, #38ef7d 100%);">
            <el-icon><CircleCheck /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ enabledCount }}</span>
            <span class="stat-label">已启用</span>
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: linear-gradient(135deg, #f093fb 0%, #f5576c 100%);">
            <el-icon><CircleClose /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ records.length - enabledCount }}</span>
            <span class="stat-label">已禁用</span>
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: linear-gradient(135deg, #4facfe 0%, #00f2fe 100%);">
            <el-icon><Collection /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ typeCount }}</span>
            <span class="stat-label">记录类型</span>
          </div>
        </div>
      </el-col>
    </el-row>

    <!-- 记录表格 -->
    <el-card class="table-card" shadow="never">
      <div class="table-wrapper">
        <el-table :data="records" v-loading="loading" stripe class="custom-table">
          <el-table-column prop="id" label="ID" width="70" />
          <el-table-column prop="name" label="域名" min-width="180">
            <template #default="{ row }">
              <span class="domain-name">{{ row.name }}</span>
            </template>
          </el-table-column>
          <el-table-column prop="record_type" label="类型" width="90">
            <template #default="{ row }">
              <el-tag :type="getTypeTagType(row.record_type)" effect="plain" size="small">
                {{ row.record_type }}
              </el-tag>
            </template>
          </el-table-column>
          <el-table-column prop="value" label="值" min-width="180">
            <template #default="{ row }">
              <span class="record-value">{{ row.value }}</span>
            </template>
          </el-table-column>
          <el-table-column prop="ttl" label="TTL" width="90">
            <template #default="{ row }">
              <span class="ttl-value">{{ row.ttl }}s</span>
            </template>
          </el-table-column>
          <el-table-column prop="priority" label="优先级" width="80" class-name="hidden-xs-only" />
          <el-table-column prop="enabled" label="状态" width="80">
            <template #default="{ row }">
              <el-switch
                v-model="row.enabled"
                @change="toggleEnabled(row)"
                inline-prompt
                active-text="启"
                inactive-text="停"
              />
            </template>
          </el-table-column>
          <el-table-column label="操作" width="120" fixed="right">
            <template #default="{ row }">
              <el-button type="primary" link @click="openEditDialog(row)">
                <el-icon><Edit /></el-icon>
              </el-button>
              <el-button type="danger" link @click="confirmDelete(row)">
                <el-icon><Delete /></el-icon>
              </el-button>
            </template>
          </el-table-column>
          <template #empty>
            <el-empty description="暂无 DNS 记录" />
          </template>
        </el-table>
      </div>
    </el-card>

    <!-- 创建/编辑对话框 -->
    <el-dialog
      v-model="dialogVisible"
      :title="isEditing ? '编辑记录' : '添加记录'"
      :width="isMobile ? '90%' : '520px'"
      class="custom-dialog"
    >
      <el-form
        ref="formRef"
        :model="formData"
        :rules="formRules"
        label-width="80px"
        label-position="top"
      >
        <el-form-item label="域名" prop="name">
          <el-input v-model="formData.name" placeholder="example.com 或 *.example.com" size="large" />
          <div class="form-tip">支持泛域名，如 *.example.com 可匹配所有子域名</div>
        </el-form-item>
        <el-row :gutter="16">
          <el-col :xs="24" :sm="12">
            <el-form-item label="类型" prop="record_type">
              <el-select v-model="formData.record_type" placeholder="选择记录类型" size="large" style="width: 100%">
                <el-option
                  v-for="type in recordTypes"
                  :key="type"
                  :label="type"
                  :value="type"
                />
              </el-select>
            </el-form-item>
          </el-col>
          <el-col :xs="24" :sm="12">
            <el-form-item label="TTL (秒)" prop="ttl">
              <el-input-number v-model="formData.ttl" :min="0" :max="86400" size="large" style="width: 100%" />
            </el-form-item>
          </el-col>
        </el-row>
        <el-form-item label="值" prop="value">
          <el-input
            v-model="formData.value"
            :placeholder="getValuePlaceholder(formData.record_type)"
            size="large"
          />
        </el-form-item>
        <el-row :gutter="16">
          <el-col :xs="24" :sm="12">
            <el-form-item label="优先级" prop="priority">
              <el-input-number v-model="formData.priority" :min="0" size="large" style="width: 100%" />
            </el-form-item>
          </el-col>
          <el-col :xs="24" :sm="12">
            <el-form-item label="状态" prop="enabled">
              <div class="flex-center" style="height: 40px">
                <el-switch v-model="formData.enabled" active-text="启用" inactive-text="禁用" size="large" />
              </div>
            </el-form-item>
          </el-col>
        </el-row>
      </el-form>
      <template #footer>
        <el-button @click="dialogVisible = false" size="large">取消</el-button>
        <el-button type="primary" @click="submitForm" :loading="submitting" size="large">
          {{ isEditing ? '保存修改' : '创建记录' }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, computed, onMounted } from 'vue'
import { ElMessage, ElMessageBox, type FormInstance, type FormRules } from 'element-plus'
import { Plus, Edit, Delete, Document, CircleCheck, CircleClose, Collection } from '@element-plus/icons-vue'
import api from '../api'
import { useResponsive } from '../composables/useResponsive'

const { isMobile } = useResponsive()

interface DnsRecord {
  id: number
  name: string
  record_type: string
  value: string
  ttl: number
  priority: number
  enabled: boolean
  created_at: string
  updated_at: string
}

const records = ref<DnsRecord[]>([])
const loading = ref(false)
const dialogVisible = ref(false)
const isEditing = ref(false)
const submitting = ref(false)
const formRef = ref<FormInstance>()
const editingId = ref<number | null>(null)

const recordTypes = ['A', 'AAAA', 'CNAME', 'MX', 'TXT', 'PTR', 'NS', 'SOA', 'SRV']

const enabledCount = computed(() => records.value.filter(r => r.enabled).length)
const typeCount = computed(() => new Set(records.value.map(r => r.record_type)).size)

const formData = reactive({
  name: '',
  record_type: 'A',
  value: '',
  ttl: 300,
  priority: 0,
  enabled: true
})

const formRules: FormRules = {
  name: [
    { required: true, message: '请输入域名', trigger: 'blur' },
    { max: 255, message: '域名长度不能超过255个字符', trigger: 'blur' }
  ],
  record_type: [
    { required: true, message: '请选择记录类型', trigger: 'change' }
  ],
  value: [
    { required: true, message: '请输入记录值', trigger: 'blur' }
  ],
  ttl: [
    { required: true, message: '请输入TTL', trigger: 'blur' }
  ]
}

function getTypeTagType(type: string): string {
  const types: Record<string, string> = {
    A: '',
    AAAA: 'success',
    CNAME: 'warning',
    MX: 'danger',
    TXT: 'info',
    PTR: '',
    NS: 'success',
    SOA: 'warning',
    SRV: 'danger'
  }
  return types[type] || ''
}

function getValuePlaceholder(type: string): string {
  const placeholders: Record<string, string> = {
    A: '192.168.1.1',
    AAAA: '2001:db8::1',
    CNAME: 'target.example.com',
    MX: 'mail.example.com',
    TXT: 'v=spf1 include:example.com ~all',
    PTR: 'host.example.com',
    NS: 'ns1.example.com',
    SOA: 'ns1.example.com admin.example.com',
    SRV: '10 5 5060 sipserver.example.com'
  }
  return placeholders[type] || ''
}

async function fetchRecords() {
  loading.value = true
  try {
    const response = await api.get('/api/records')
    records.value = response.data.data
  } catch (error: any) {
    ElMessage.error(error.response?.data?.message || '获取记录失败')
  } finally {
    loading.value = false
  }
}

function resetForm() {
  formData.name = ''
  formData.record_type = 'A'
  formData.value = ''
  formData.ttl = 300
  formData.priority = 0
  formData.enabled = true
  editingId.value = null
}

function openCreateDialog() {
  isEditing.value = false
  resetForm()
  dialogVisible.value = true
}

function openEditDialog(record: DnsRecord) {
  isEditing.value = true
  editingId.value = record.id
  formData.name = record.name
  formData.record_type = record.record_type
  formData.value = record.value
  formData.ttl = record.ttl
  formData.priority = record.priority
  formData.enabled = record.enabled
  dialogVisible.value = true
}

async function submitForm() {
  if (!formRef.value) return

  await formRef.value.validate(async (valid) => {
    if (!valid) return

    submitting.value = true
    try {
      if (isEditing.value && editingId.value) {
        await api.put(`/api/records/${editingId.value}`, formData)
        ElMessage.success('记录更新成功')
      } else {
        await api.post('/api/records', formData)
        ElMessage.success('记录创建成功')
      }
      dialogVisible.value = false
      fetchRecords()
    } catch (error: any) {
      const message = error.response?.data?.message || '操作失败'
      ElMessage.error(message)
    } finally {
      submitting.value = false
    }
  })
}

async function toggleEnabled(record: DnsRecord) {
  try {
    await api.put(`/api/records/${record.id}`, { enabled: record.enabled })
    ElMessage.success(record.enabled ? '记录已启用' : '记录已禁用')
  } catch (error: any) {
    record.enabled = !record.enabled
    ElMessage.error(error.response?.data?.message || '操作失败')
  }
}

async function confirmDelete(record: DnsRecord) {
  try {
    await ElMessageBox.confirm(
      `确定要删除记录 "${record.name}" 吗？`,
      '确认删除',
      {
        confirmButtonText: '删除',
        cancelButtonText: '取消',
        type: 'warning'
      }
    )
    await api.delete(`/api/records/${record.id}`)
    ElMessage.success('记录删除成功')
    fetchRecords()
  } catch (error: any) {
    if (error !== 'cancel') {
      ElMessage.error(error.response?.data?.message || '删除失败')
    }
  }
}

onMounted(() => {
  fetchRecords()
})
</script>

<style scoped>
.dns-records {
  max-width: 1400px;
  margin: 0 auto;
}

/* 页面标题 */
.page-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  margin-bottom: 24px;
}

.header-left h1 {
  margin: 0 0 8px 0;
  font-size: 24px;
  font-weight: 600;
  color: #303133;
}

.subtitle {
  margin: 0;
  font-size: 14px;
  color: #909399;
}

/* 统计卡片 */
.stats-row {
  margin-bottom: 24px;
}

.stat-card {
  background: #fff;
  border-radius: 12px;
  padding: 20px;
  display: flex;
  align-items: center;
  gap: 16px;
  box-shadow: 0 2px 12px rgba(0, 0, 0, 0.04);
  transition: transform 0.3s, box-shadow 0.3s;
}

.stat-card:hover {
  transform: translateY(-2px);
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.08);
}

.stat-icon {
  width: 48px;
  height: 48px;
  border-radius: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  font-size: 24px;
}

.stat-info {
  display: flex;
  flex-direction: column;
}

.stat-value {
  font-size: 24px;
  font-weight: 600;
  color: #303133;
}

.stat-label {
  font-size: 13px;
  color: #909399;
  margin-top: 4px;
}

/* 表格卡片 */
.table-card {
  border-radius: 12px;
  border: none;
}

.table-card :deep(.el-card__body) {
  padding: 0;
}

.custom-table {
  border-radius: 12px;
}

.custom-table :deep(.el-table__header th) {
  background: #f8f9fa;
  color: #606266;
  font-weight: 600;
}

.domain-name {
  font-weight: 500;
  color: #303133;
}

.record-value {
  font-family: 'Monaco', 'Menlo', monospace;
  font-size: 13px;
  color: #606266;
}

.ttl-value {
  color: #909399;
}

/* 对话框 */
.custom-dialog :deep(.el-dialog__header) {
  border-bottom: 1px solid #f0f0f0;
  padding: 20px 24px;
}

.custom-dialog :deep(.el-dialog__body) {
  padding: 24px;
}

.custom-dialog :deep(.el-dialog__footer) {
  border-top: 1px solid #f0f0f0;
  padding: 16px 24px;
}

/* 表格包装器 */
.table-wrapper {
  overflow-x: auto;
  -webkit-overflow-scrolling: touch;
}

/* 响应式 */
@media (max-width: 768px) {
  .page-header {
    flex-direction: column;
    align-items: stretch;
    gap: 16px;
  }

  .header-left h1 {
    font-size: 20px;
  }

  .stat-card {
    padding: 12px;
    gap: 10px;
  }

  .stat-icon {
    width: 36px;
    height: 36px;
    font-size: 18px;
    border-radius: 8px;
  }

  .stat-value {
    font-size: 18px;
  }

  .stat-label {
    font-size: 12px;
  }

  .custom-dialog :deep(.el-dialog__body) {
    padding: 16px;
  }

  .flex-center {
    display: flex;
    align-items: center;
  }
}

/* 表单提示 */
.form-tip {
  font-size: 12px;
  color: #909399;
  margin-top: 4px;
}
</style>
