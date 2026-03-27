{{- define "tf-runner.chart" -}}
{{- .Chart.Name -}}
{{- end -}}

{{- define "tf-runner.name" -}}
{{- include "tf-runner.chart" . -}}
{{- end -}}

{{- define "tf-runner.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- printf "%s" .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s" .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "tf-runner.labels" -}}
app.kubernetes.io/name: {{ include "tf-runner.chart" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "tf-runner.selectorLabels" -}}
app.kubernetes.io/name: {{ include "tf-runner.chart" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "tf-runner.nodeLabels" -}}
{{- $root := index . "root" -}}
{{- $index := index . "index" -}}
app.kubernetes.io/name: {{ include "tf-runner.chart" $root }}
app.kubernetes.io/instance: {{ $root.Release.Name }}
testing-framework/component: node
testing-framework/node-index: "{{ $index }}"
{{- end -}}
