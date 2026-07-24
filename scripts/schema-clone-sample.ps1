#Requires -Version 5.1
<#
.SYNOPSIS
  用已有 dbx-cli 导出「表结构 DDL + 样例 SELECT/INSERT」（本机脚本，非产品 report）。

.DESCRIPTION
  1) schema list 列出表
  2) schema describe 拿字段（名/类型/可空/PK/默认/注释）→ 生成 CREATE TABLE
  3) 对有数据的表：按 PK 或时间列 DESC 取样例（默认 20 行），输出样例 SELECT + INSERT

.PARAMETER Connection
  连接名或 connections list 序号（如 whatsapp_call 或 12）

.PARAMETER Database
  可选，覆盖连接默认库（-d）

.PARAMETER Schema
  可选，schema（-s；Postgres 等）

.PARAMETER SampleRows
  样例行数上限，默认 20

.PARAMETER OutDir
  输出目录；默认 .\schema-clone-<conn>-<stamp>\
  写入 schema-clone.sql（结构+样例）

.PARAMETER Stdout
  只打印到 stdout，不写文件

.PARAMETER DbxCli
  dbx-cli 路径；默认依次尝试 target\release\dbx-cli.exe、仓库根 dbx-cli.exe、PATH

.EXAMPLE
  .\scripts\schema-clone-sample.ps1 -Connection whatsapp_call

.EXAMPLE
  .\scripts\schema-clone-sample.ps1 -Connection 12 -Database mydb -SampleRows 10 -OutDir .\out\clone
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Connection,

    [string]$Database = "",
    [string]$Schema = "",
    [ValidateRange(1, 500)]
    [int]$SampleRows = 20,
    [string]$OutDir = "",
    [switch]$Stdout,
    [string]$DbxCli = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-DbxCli {
    param([string]$Explicit)
    if ($Explicit -and (Test-Path -LiteralPath $Explicit)) {
        return (Resolve-Path -LiteralPath $Explicit).Path
    }
    $root = Split-Path -Parent $PSScriptRoot
    $candidates = @(
        (Join-Path $root "target\release\dbx-cli.exe"),
        (Join-Path $root "dbx-cli.exe"),
        "dbx-cli.exe",
        "dbx-cli"
    )
    foreach ($c in $candidates) {
        if ($c -notmatch '[\\/]' ) {
            $cmd = Get-Command $c -ErrorAction SilentlyContinue
            if ($cmd) { return $cmd.Source }
            continue
        }
        if (Test-Path -LiteralPath $c) {
            return (Resolve-Path -LiteralPath $c).Path
        }
    }
    throw "找不到 dbx-cli。请先编译 target\release\dbx-cli.exe，或用 -DbxCli 指定路径。"
}

function Invoke-DbxJson {
    param(
        [string]$Cli,
        [string[]]$CliArgs
    )
    $all = @($CliArgs) + @("-j", "-q")
    $raw = & $Cli @all 2>&1
    $exit = $LASTEXITCODE
    $text = ($raw | ForEach-Object { "$_" }) -join "`n"
    $text = $text.Trim()
    if ($exit -ne 0) {
        if (-not $text) { $text = "dbx-cli exit $exit" }
        throw "dbx-cli 失败: $($CliArgs -join ' ')`n$text"
    }
    if (-not $text) { throw "dbx-cli 无输出: $($CliArgs -join ' ')" }
    # 进度行偶发混入时，从首个 { 起解析 JSON
    $brace = $text.IndexOf('{')
    if ($brace -gt 0) { $text = $text.Substring($brace) }
    return ($text | ConvertFrom-Json)
}

function Get-QuoteStyle {
    param([string]$DbType)
    $t = ""
    if ($null -ne $DbType) { $t = $DbType.ToLowerInvariant() }
    if ($t -match '^(mysql|mariadb|doris|starrocks|manticoresearch|tidb)$') { return "mysql" }
    if ($t -match '^(sqlite|rqlite)$') { return "sqlite" }
    if ($t -match '^(sqlserver|mssql)$') { return "mssql" }
    return "pg"
}

function Quote-Ident {
    param([string]$Name, [string]$Style)
    switch ($Style) {
        "mysql" {
            $escaped = $Name -replace '`', '``'
            return '`' + $escaped + '`'
        }
        "sqlite" { return '"' + ($Name -replace '"', '""') + '"' }
        "mssql" { return "[" + ($Name -replace ']', ']]') + "]" }
        default { return '"' + ($Name -replace '"', '""') + '"' }
    }
}

function Quote-Lit {
    param($Value)
    if ($null -eq $Value) { return "NULL" }
    if ($Value -is [bool]) { return $(if ($Value) { "1" } else { "0" }) }
    if ($Value -is [int] -or $Value -is [long] -or $Value -is [decimal] -or $Value -is [double] -or $Value -is [float]) {
        return ([string]$Value)
    }
    # ConvertFrom-Json may produce PSCustomObject / DateTime / string
    $s = if ($Value -is [datetime]) {
        $Value.ToString("yyyy-MM-dd HH:mm:ss.fff")
    } else {
        [string]$Value
    }
    return "'" + ($s -replace "'", "''") + "'"
}

function Resolve-OrderColumn {
    param($Columns)
    $pks = @($Columns | Where-Object { $_.is_primary_key -eq $true } | ForEach-Object { $_.name })
    if ($pks.Count -eq 1) { return $pks[0] }
    if ($pks.Count -gt 1) {
        # 复合主键：用第一个 PK（通常是自增/时间友好的）
        return $pks[0]
    }
    $timeHints = @(
        "updated_at", "update_time", "modified_at", "modify_time", "gmt_modified",
        "created_at", "create_time", "gmt_create", "insert_time", "timestamp", "ts", "id"
    )
    $names = @($Columns | ForEach-Object { $_.name })
    foreach ($hint in $timeHints) {
        $hit = $names | Where-Object { $_.ToLowerInvariant() -eq $hint } | Select-Object -First 1
        if ($hit) { return $hit }
    }
    foreach ($col in $Columns) {
        $dt = ([string]$col.data_type).ToLowerInvariant()
        if ($dt -match 'date|time|timestamp') { return $col.name }
    }
    return $null
}

function Build-CreateTable {
    param(
        [string]$Table,
        [string]$SchemaName,
        [object[]]$Columns,
        [string]$Style
    )
    $qt = Quote-Ident -Name $Table -Style $Style
    if ($SchemaName) {
        $qs = Quote-Ident -Name $SchemaName -Style $Style
        $full = "$qs.$qt"
    } else {
        $full = $qt
    }
    $lines = New-Object System.Collections.Generic.List[string]
    $pkCols = @()
    foreach ($c in $Columns) {
        $qn = Quote-Ident -Name $c.name -Style $Style
        $nullability = if ($c.is_nullable) { "NULL" } else { "NOT NULL" }
        $def = ""
        if ($null -ne $c.column_default -and [string]$c.column_default -ne "") {
            $def = " DEFAULT $($c.column_default)"
        }
        $comment = ""
        if ($c.comment) {
            $comment = " -- " + ([string]$c.comment -replace "[\r\n]+", " ")
        }
        $pkMark = ""
        if ($c.is_primary_key) {
            $pkCols += $c.name
            $pkMark = " /* PK */"
        }
        $lines.Add("  $qn $($c.data_type) $nullability$def$pkMark$comment")
    }
    $pkSql = ""
    if ($pkCols.Count -gt 0) {
        $quotedPk = ($pkCols | ForEach-Object { Quote-Ident -Name $_ -Style $Style }) -join ", "
        $pkSql = ",`n  PRIMARY KEY ($quotedPk)"
    }
    $body = ($lines -join ",`n")
    $ddl = "-- structure: $Table`nCREATE TABLE IF NOT EXISTS $full (`n$body$pkSql`n);"
    return $ddl
}

function Build-SampleInsert {
    param(
        [string]$Table,
        [string]$SchemaName,
        [object]$QueryResult,
        [string]$Style,
        [string]$SampleSelectSql
    )
    $qt = Quote-Ident -Name $Table -Style $Style
    if ($SchemaName) {
        $full = "$(Quote-Ident -Name $SchemaName -Style $Style).$qt"
    } else {
        $full = $qt
    }
    $sb = New-Object System.Text.StringBuilder
    [void]$sb.AppendLine("-- sample select used:")
    [void]$sb.AppendLine("-- $SampleSelectSql")
    $rows = @($QueryResult.rows)
    if ($rows.Count -eq 0) {
        [void]$sb.AppendLine("-- (no sample rows)")
        return $sb.ToString()
    }
    $cols = @($QueryResult.columns)
    $colList = ($cols | ForEach-Object { Quote-Ident -Name $_ -Style $Style }) -join ", "
    [void]$sb.AppendLine("-- sample rows: $($rows.Count)")
    foreach ($row in $rows) {
        $vals = foreach ($col in $cols) {
            $prop = $row.PSObject.Properties[$col]
            $v = if ($null -ne $prop) { $prop.Value } else { $null }
            Quote-Lit -Value $v
        }
        [void]$sb.AppendLine("INSERT INTO $full ($colList) VALUES ($($vals -join ', '));")
    }
    return $sb.ToString()
}

function Qualify-TableRef {
    param([string]$Table, [string]$SchemaName, [string]$Style)
    $qt = Quote-Ident -Name $Table -Style $Style
    if ($SchemaName) {
        return "$(Quote-Ident -Name $SchemaName -Style $Style).$qt"
    }
    return $qt
}

# ---- main ----
$cli = Resolve-DbxCli -Explicit $DbxCli
Write-Host "[schema-clone] using: $cli"

$connList = Invoke-DbxJson -Cli $cli -CliArgs @("connections", "list")
$connMeta = $null
if ($Connection -match '^\d+$') {
    $connMeta = $connList.connections | Where-Object { [string]$_.index -eq $Connection } | Select-Object -First 1
} else {
    $connMeta = $connList.connections | Where-Object { $_.name -eq $Connection } | Select-Object -First 1
    if (-not $connMeta) {
        $connMeta = $connList.connections | Where-Object { $_.name -like "*$Connection*" } | Select-Object -First 1
    }
}
if (-not $connMeta) {
    throw "连接未找到: $Connection（先跑 dbx-cli connections list）"
}
$dbType = [string]$connMeta.type
$style = Get-QuoteStyle -DbType $dbType
$connRef = [string]$connMeta.name
Write-Host "[schema-clone] connection=$connRef type=$dbType quote=$style sampleRows=$SampleRows"

$common = @()
if ($Database) { $common += @("-d", $Database) }
if ($Schema) { $common += @("-s", $Schema) }

$listArgs = @("schema", "list", $connRef) + $common
$tablesJson = Invoke-DbxJson -Cli $cli -CliArgs $listArgs
$tables = @($tablesJson.tables)
if ($tables.Count -eq 0) {
    Write-Warning "没有表。"
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
if (-not $OutDir) {
    $safeName = ($connRef -replace '[^\w\-]+', '_')
    $OutDir = Join-Path (Get-Location) "schema-clone-$safeName-$stamp"
}
if (-not $Stdout) {
    New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
}

$sqlParts = New-Object System.Collections.Generic.List[string]
$header = @"
-- Generated by scripts/schema-clone-sample.ps1
-- connection: $connRef
-- db_type: $dbType
-- database: $(if ($Database) { $Database } else { $connMeta.database })
-- schema: $Schema
-- sample_rows: $SampleRows
-- generated_at: $(Get-Date -Format o)
-- NOTE: DDL from describe columns; indexes/FKs/engine options may be incomplete.
"@
$sqlParts.Add($header)

$schemaName = if ($Schema) { $Schema } else { "" }
$tableCount = 0
$sampleTableCount = 0

foreach ($t in $tables) {
    $tableName = [string]$t.name
    $tableType = [string]$t.type
    # 跳过明显非基表（视图等仍可导出结构，但不取样例）
    $isBase = ($tableType -eq "" -or $tableType -match '(?i)table|base') -and ($tableType -notmatch '(?i)view')
    Write-Host "[schema-clone] ($($tableCount + 1)/$($tables.Count)) $tableName [$tableType]"

    $descArgs = @("schema", "describe", $connRef, $tableName) + $common
    try {
        $desc = Invoke-DbxJson -Cli $cli -CliArgs $descArgs
    } catch {
        $sqlParts.Add("-- ERROR describe $tableName : $_")
        Write-Warning "describe failed: $tableName - $_"
        continue
    }
    $columns = @($desc.columns)
    if ($columns.Count -eq 0) {
        $sqlParts.Add("-- skip empty describe: $tableName")
        continue
    }

    $sqlParts.Add("")
    $sqlParts.Add( (Build-CreateTable -Table $tableName -SchemaName $schemaName -Columns $columns -Style $style) )
    $tableCount++

    if (-not $isBase) {
        $sqlParts.Add("-- skip sample (not base table): $tableName")
        continue
    }

    $orderCol = Resolve-OrderColumn -Columns $columns
    $fullRef = Qualify-TableRef -Table $tableName -SchemaName $schemaName -Style $style
    if ($orderCol) {
        $qo = Quote-Ident -Name $orderCol -Style $style
        $sampleSql = "SELECT * FROM $fullRef ORDER BY $qo DESC LIMIT $SampleRows"
    } else {
        $sampleSql = "SELECT * FROM $fullRef LIMIT $SampleRows"
    }

    $queryArgs = @("query", $connRef, $sampleSql, "--limit", "$SampleRows", "-t", "60s") + $common
    try {
        $q = Invoke-DbxJson -Cli $cli -CliArgs $queryArgs
    } catch {
        # 部分引擎不支持 LIMIT / ORDER BY 语法：退化无序 LIMIT（再失败则跳过）
        if ($orderCol) {
            $sampleSql = "SELECT * FROM $fullRef LIMIT $SampleRows"
            $queryArgs = @("query", $connRef, $sampleSql, "--limit", "$SampleRows", "-t", "60s") + $common
            try {
                $q = Invoke-DbxJson -Cli $cli -CliArgs $queryArgs
            } catch {
                $sqlParts.Add("-- ERROR sample $tableName : $_")
                Write-Warning "sample failed: $tableName - $_"
                continue
            }
        } else {
            $sqlParts.Add("-- ERROR sample $tableName : $_")
            Write-Warning "sample failed: $tableName - $_"
            continue
        }
    }

    $rowCount = 0
    if ($null -ne $q.row_count) { $rowCount = [int]$q.row_count }
    elseif ($q.rows) { $rowCount = @($q.rows).Count }

    if ($rowCount -le 0) {
        $sqlParts.Add("-- no data: $tableName")
        continue
    }

    $sqlParts.Add( (Build-SampleInsert -Table $tableName -SchemaName $schemaName -QueryResult $q -Style $style -SampleSelectSql $sampleSql) )
    $sampleTableCount++
}

$sqlParts.Add("")
$sqlParts.Add("-- done: tables_with_ddl=$tableCount tables_with_samples=$sampleTableCount")

$finalSql = ($sqlParts -join "`n") + "`n"

if ($Stdout) {
    Write-Output $finalSql
} else {
    $outFile = Join-Path $OutDir "schema-clone.sql"
    Set-Content -LiteralPath $outFile -Value $finalSql -Encoding UTF8
    Write-Host "[schema-clone] wrote: $outFile"
    Write-Host "[schema-clone] tables=$tableCount samples=$sampleTableCount"
}
