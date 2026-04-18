# download-bible-data.ps1
# Downloads public-domain Bible data files into .ghost/bible-data/
# for use with `claw bible-ingest`.
#
# Usage (from Git Bash):
#   powershell -ExecutionPolicy Bypass -File scripts/download-bible-data.ps1
#
# Downloads KJV from thiagobodruk/bible (nested format) and converts
# to the flat [{book, chapter, verse, text}] format the ingestion expects.

$ErrorActionPreference = "Stop"
$dataDir = ".ghost\bible-data"

if (-not (Test-Path $dataDir)) {
    New-Item -ItemType Directory -Path $dataDir -Force | Out-Null
    Write-Host "Created $dataDir"
}

# Book abbreviation -> full name mapping (thiagobodruk/bible uses these abbrevs)
$bookNames = @{
    "gn"="Genesis"; "ex"="Exodus"; "lv"="Leviticus"; "nm"="Numbers"; "dt"="Deuteronomy"
    "js"="Joshua"; "jdgs"="Judges"; "rt"="Ruth"; "1sm"="1 Samuel"; "2sm"="2 Samuel"
    "1kgs"="1 Kings"; "2kgs"="2 Kings"; "1ch"="1 Chronicles"; "2ch"="2 Chronicles"
    "ezr"="Ezra"; "ne"="Nehemiah"; "et"="Esther"; "job"="Job"; "ps"="Psalms"
    "prv"="Proverbs"; "ec"="Ecclesiastes"; "so"="Song of Solomon"; "is"="Isaiah"
    "jr"="Jeremiah"; "lm"="Lamentations"; "ez"="Ezekiel"; "dn"="Daniel"; "ho"="Hosea"
    "jl"="Joel"; "am"="Amos"; "ob"="Obadiah"; "jn"="Jonah"; "mc"="Micah"; "na"="Nahum"
    "hk"="Habakkuk"; "zp"="Zephaniah"; "hg"="Haggai"; "zc"="Zechariah"; "ml"="Malachi"
    "mt"="Matthew"; "mk"="Mark"; "lk"="Luke"; "jo"="John"; "act"="Acts"; "rm"="Romans"
    "1co"="1 Corinthians"; "2co"="2 Corinthians"; "gl"="Galatians"; "eph"="Ephesians"
    "ph"="Philippians"; "cl"="Colossians"; "1ts"="1 Thessalonians"; "2ts"="2 Thessalonians"
    "1tm"="1 Timothy"; "2tm"="2 Timothy"; "tt"="Titus"; "phm"="Philemon"; "hb"="Hebrews"
    "jm"="James"; "1pe"="1 Peter"; "2pe"="2 Peter"; "1jo"="1 John"; "2jo"="2 John"
    "3jo"="3 John"; "jd"="Jude"; "re"="Revelation"
}

function Download-IfMissing {
    param([string]$Url, [string]$OutFile)
    $path = Join-Path $dataDir $OutFile
    if (Test-Path $path) {
        Write-Host "  [skip] $OutFile already exists"
        return $false
    }
    Write-Host "  [download] $OutFile ..."
    try {
        Invoke-WebRequest -Uri $Url -OutFile $path -UseBasicParsing
        Write-Host "  [done] $OutFile"
        return $true
    } catch {
        Write-Host "  [FAIL] $OutFile -- $($_.Exception.Message)"
        return $false
    }
}

# Convert nested [{abbrev, chapters: [[verse_text]]}] to flat [{book, chapter, verse, text}]
function Convert-NestedToFlat {
    param([string]$InFile, [string]$OutFile, [hashtable]$Names)
    $inPath = Join-Path $dataDir $InFile
    $outPath = Join-Path $dataDir $OutFile

    if (-not (Test-Path $inPath)) {
        Write-Host "  [skip] $InFile not found, cannot convert"
        return
    }
    if (Test-Path $outPath) {
        Write-Host "  [skip] $OutFile already exists"
        return
    }

    Write-Host "  [convert] $InFile -> $OutFile ..."
    $raw = Get-Content $inPath -Raw -Encoding UTF8
    $books = $raw | ConvertFrom-Json

    $verses = [System.Collections.ArrayList]::new()
    foreach ($book in $books) {
        $abbrev = $book.abbrev
        $fullName = if ($Names.ContainsKey($abbrev)) { $Names[$abbrev] } else { $abbrev }

        for ($ch = 0; $ch -lt $book.chapters.Count; $ch++) {
            $chapter = $book.chapters[$ch]
            for ($v = 0; $v -lt $chapter.Count; $v++) {
                $text = $chapter[$v] -replace '\{[^}]*\}', ''
                $text = $text.Trim()
                if ($text) {
                    [void]$verses.Add([PSCustomObject]@{
                        book    = $fullName
                        chapter = $ch + 1
                        verse   = $v + 1
                        text    = $text
                    })
                }
            }
        }
    }

    $json = $verses | ConvertTo-Json -Depth 3 -Compress
    [System.IO.File]::WriteAllText($outPath, $json, [System.Text.Encoding]::UTF8)
    Write-Host "  [done] $OutFile ($($verses.Count) verses)"
}

Write-Host ""
Write-Host "=== Bible Data Downloader ==="
Write-Host "Target: $dataDir"
Write-Host ""

# --- KJV text (required) ---
Write-Host "[1/7] KJV verse-aligned JSON"
$downloaded = Download-IfMissing `
    -Url "https://raw.githubusercontent.com/thiagobodruk/bible/master/json/en_kjv.json" `
    -OutFile "en_kjv_raw.json"

Convert-NestedToFlat -InFile "en_kjv_raw.json" -OutFile "kjv.json" -Names $bookNames

# --- WEB text (optional) ---
Write-Host "[2/7] WEB (World English Bible) JSON"
Write-Host "  [info] thiagobodruk/bible does not include WEB."
Write-Host "         If you have a WEB source, place it as .ghost/bible-data/web.json"
Write-Host "         Format: [{book, chapter, verse, text}, ...]"

# --- Hebrew WLC (optional) ---
Write-Host "[3/7] Hebrew (Westminster Leningrad Codex)"
Write-Host "  [manual] hebrew-wlc.json requires conversion from OSIS XML"
Write-Host "           Source: https://github.com/openscriptures/morphhb"
Write-Host "           Format: [{book, chapter, verse, text, strongs[], morphology{}}]"

# --- Greek UGNT (optional) ---
Write-Host "[4/7] Greek (unfoldingWord Greek NT)"
Write-Host "  [manual] greek-ugnt.json requires conversion from USFM"
Write-Host "           Source: https://github.com/unfoldingWord/en_ugnt"
Write-Host "           Format: [{book, chapter, verse, text, strongs[], morphology{}}]"

# --- Strong's lexicon (optional) ---
Write-Host "[5/7] Strong's Concordance (Hebrew + Greek)"
Write-Host "  [manual] strongs-hebrew.json and strongs-greek.json"
Write-Host "           Source: https://github.com/openscriptures/strongs"
Write-Host "           Format: [{strongs_id, original_word, transliteration, definition, root, semantic_range[]}]"

# --- Cross-references (optional) ---
Write-Host "[6/7] Treasury of Scripture Knowledge cross-references"
Write-Host "  [manual] cross-refs.tsv"
Write-Host "           Source: https://www.openbible.info/labs/cross-references/"
Write-Host "           Format: TSV with columns: source_book source_chapter source_verse target_book target_chapter target_verse rel_type"

# --- Pericopes (optional) ---
Write-Host "[7/7] Pericope boundaries"
Write-Host "  [manual] pericopes.json"
Write-Host "           Format: [{title, start_book, start_chapter, start_verse, end_book, end_chapter, end_verse, genre}]"

Write-Host ""
Write-Host "=== Summary ==="
Write-Host "Files in $dataDir :"
Get-ChildItem $dataDir | ForEach-Object { Write-Host "  $($_.Name) ($([math]::Round($_.Length / 1KB, 1)) KB)" }
Write-Host ""
Write-Host "Once kjv.json exists, run: claw bible-ingest"
