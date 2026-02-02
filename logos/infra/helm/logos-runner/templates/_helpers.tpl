{{- define "logos-runner.chart" -}}
{{- .Chart.Name -}}
{{- end -}}

{{- define "logos-runner.name" -}}
{{- include "logos-runner.chart" . -}}
{{- end -}}

{{- define "logos-runner.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- printf "%s" .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s" .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "logos-runner.labels" -}}
app.kubernetes.io/name: {{ include "logos-runner.chart" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "logos-runner.selectorLabels" -}}
app.kubernetes.io/name: {{ include "logos-runner.chart" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "logos-runner.nodeLabels" -}}
{{- $root := index . "root" -}}
{{- $index := index . "index" -}}
app.kubernetes.io/name: {{ include "logos-runner.chart" $root }}
app.kubernetes.io/instance: {{ $root.Release.Name }}
logos/logical-role: node
logos/node-index: "{{ $index }}"
{{- end -}}
