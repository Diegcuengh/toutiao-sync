param(
    [Parameter(Mandatory = $true)]
    [string]$SourceDir,

    [Parameter(Mandatory = $true)]
    [string]$OutputDir
)

$ErrorActionPreference = 'Stop'

function Set-RangeStyle {
    param(
        $Range,
        [string]$FontName,
        [double]$Size,
        [int]$Bold,
        [int]$Align,
        [int]$IndentChars,
        [double]$LineSpacing
    )

    try { $Range.Font.Name = $FontName } catch {}
    try { $Range.Font.NameFarEast = $FontName } catch {}
    try { $Range.Font.Size = $Size } catch {}
    try { $Range.Font.Bold = $Bold } catch {}
    try { $Range.ParagraphFormat.Alignment = $Align } catch {}
    try { $Range.ParagraphFormat.CharacterUnitFirstLineIndent = $IndentChars } catch {}
    try { $Range.ParagraphFormat.LineSpacingRule = 4 } catch {}
    try { $Range.ParagraphFormat.LineSpacing = $LineSpacing } catch {}
    try { $Range.ParagraphFormat.SpaceBefore = 0 } catch {}
    try { $Range.ParagraphFormat.SpaceAfter = 0 } catch {}
}

function Get-CleanText {
    param($Paragraph)
    return $Paragraph.Range.Text.Replace("`r", '').Replace(([string][char]7), '').Trim()
}

function Format-Document {
    param($Doc)

    $alignLeft = 0
    $alignCenter = 1
    $alignRight = 2
    $fullWidthColon = [string][char]0xFF1A

    $Doc.PageSetup.TopMargin = 105
    $Doc.PageSetup.BottomMargin = 99
    $Doc.PageSetup.LeftMargin = 79
    $Doc.PageSetup.RightMargin = 74

    Set-RangeStyle -Range $Doc.Content -FontName 'FangSong_GB2312' -Size 16 -Bold 0 -Align $alignLeft -IndentChars 2 -LineSpacing 28

    $nonEmpty = @()
    for ($i = 1; $i -le $Doc.Paragraphs.Count; $i++) {
        $p = $Doc.Paragraphs.Item($i)
        if ($null -eq $p) { continue }
        $text = Get-CleanText -Paragraph $p
        if ($text) {
            $nonEmpty += [pscustomobject]@{ Index = $i; Text = $text }
        }
    }

    foreach ($item in $nonEmpty) {
        $p = $Doc.Paragraphs.Item($item.Index)
        if ($null -eq $p) { continue }
        $text = $item.Text

        Set-RangeStyle -Range $p.Range -FontName 'FangSong_GB2312' -Size 16 -Bold 0 -Align $alignLeft -IndentChars 2 -LineSpacing 28

        if ($item.Index -eq $nonEmpty[0].Index) {
            Set-RangeStyle -Range $p.Range -FontName 'SimSun' -Size 18 -Bold 1 -Align $alignCenter -IndentChars 0 -LineSpacing 28
            continue
        }

        if ($nonEmpty.Count -ge 2 -and $item.Index -eq $nonEmpty[1].Index) {
            Set-RangeStyle -Range $p.Range -FontName 'SimHei' -Size 22 -Bold 1 -Align $alignCenter -IndentChars 0 -LineSpacing 28
            continue
        }

        if ($nonEmpty.Count -ge 3 -and $item.Index -eq $nonEmpty[2].Index) {
            Set-RangeStyle -Range $p.Range -FontName 'FangSong_GB2312' -Size 16 -Bold 0 -Align $alignCenter -IndentChars 0 -LineSpacing 28
            continue
        }

        if ($text.Length -le 40 -and (($text.IndexOf(':') -ge 0) -or ($text.IndexOf($fullWidthColon) -ge 0))) {
            Set-RangeStyle -Range $p.Range -FontName 'FangSong_GB2312' -Size 16 -Bold 0 -Align $alignLeft -IndentChars 0 -LineSpacing 28
            continue
        }

        if ($text -match '^[0-9]+\.') {
            Set-RangeStyle -Range $p.Range -FontName 'SimHei' -Size 16 -Bold 1 -Align $alignLeft -IndentChars 0 -LineSpacing 28
            continue
        }
    }

    $tail = $nonEmpty | Select-Object -Last 4
    foreach ($item in $tail) {
        if ($item.Text.Length -le 20) {
            $p = $Doc.Paragraphs.Item($item.Index)
            if ($null -eq $p) { continue }
            Set-RangeStyle -Range $p.Range -FontName 'FangSong_GB2312' -Size 16 -Bold 0 -Align $alignRight -IndentChars 0 -LineSpacing 28
        }
    }
}

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
$workDir = Join-Path $env:TEMP ('legal_doc_pdf_' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $workDir | Out-Null

$files = @(Get-ChildItem -LiteralPath $SourceDir -Filter *.doc | Sort-Object Name)
for ($idx = 0; $idx -lt $files.Count; $idx++) {
    $file = $files[$idx]
    $tmpPath = Join-Path $workDir ('{0:D2}.doc' -f ($idx + 1))
    Copy-Item -LiteralPath $file.FullName -Destination $tmpPath -Force

    $word = $null
    $doc = $null
    try {
        $word = New-Object -ComObject Word.Application
        $word.Visible = $false
        $word.DisplayAlerts = 0

        $doc = $word.Documents.Open($tmpPath, $false, $false)
        Format-Document -Doc $doc

        $pdfName = [System.IO.Path]::GetFileNameWithoutExtension($file.Name) + '.pdf'
        $pdfPath = Join-Path $OutputDir $pdfName
        $doc.ExportAsFixedFormat($pdfPath, 17)
        Write-Output ("OK`t{0}" -f $pdfPath)
    }
    finally {
        if ($doc -ne $null) {
            try { $doc.Close([ref]0) } catch {}
            try { [System.Runtime.InteropServices.Marshal]::ReleaseComObject($doc) | Out-Null } catch {}
        }
        if ($word -ne $null) {
            try { $word.Quit() } catch {}
            try { [System.Runtime.InteropServices.Marshal]::ReleaseComObject($word) | Out-Null } catch {}
        }
    }
}
try { Remove-Item -LiteralPath $workDir -Recurse -Force } catch {}
