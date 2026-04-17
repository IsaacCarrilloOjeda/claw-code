# download-bible-data.ps1
# Downloads public-domain Bible data files into .ghost/bible-data/
# for use with `claw bible-ingest`.
#
# Usage: .\scripts\download-bible-data.ps1
#
# Most sources are GitHub repos with verse-aligned JSON.
# Some URLs may need updating -- check the TODOs below.

$ErrorActionPreference = "Stop"
$dataDir = ".ghost\bible-data"

if (-not (Test-Path $dataDir)) {
    New-Item -ItemType Directory -Path $dataDir -Force | Out-Null
    Write-Host "Created $dataDir"
}

function Download-IfMissing {
    param([string]$Url, [string]$OutFile)
    $path = Join-Path $dataDir $OutFile
    if (Test-Path $path) {
        Write-Host "  [skip] $OutFile already exists"
        return
    }
    Write-Host "  [download] $OutFile ..."
    try {
        Invoke-WebRequest -Uri $Url -OutFile $path -UseBasicParsing
        Write-Host "  [done] $OutFile"
    } catch {
        Write-Host "  [FAIL] $OutFile -- $($_.Exception.Message)"
        Write-Host "         You may need to download this manually."
    }
}

Write-Host ""
Write-Host "=== Bible Data Downloader ==="
Write-Host "Target: $dataDir"
Write-Host ""

# --- KJV text (required) ---
Write-Host "[1/7] KJV verse-aligned JSON"
# TODO: update URL -- needs a JSON array of {book, chapter, verse, text}
# Options:
#   https://github.com/thiagobodruk/bible (has multiple translations)
#   https://github.com/aruljohn/Bible-kjv (verse-per-line JSON)
#   https://raw.githubusercontent.com/aruljohn/Bible-kjv/master/kjv.json
Download-IfMissing `
    -Url "https://raw.githubusercontent.com/aruljohn/Bible-kjv/master/kjv.json" `
    -OutFile "kjv.json"

# --- WEB text (optional) ---
Write-Host "[2/7] WEB (World English Bible) JSON"
# TODO: update URL -- needs same format as KJV
# The WEB is public domain. Check:
#   https://github.com/nicholasgasior/bible-api-go/tree/master/data
#   https://ebible.org/find/details.php?id=engwebp
Download-IfMissing `
    -Url "https://raw.githubusercontent.com/nicholasgasior/bible-api-go/master/data/web.json" `
    -OutFile "web.json"

# --- Hebrew WLC (optional) ---
Write-Host "[3/7] Hebrew (Westminster Leningrad Codex)"
# TODO: update URL -- needs {book, chapter, verse, text, strongs[], morphology{}}
# Source: https://github.com/openscriptures/morphhb
# You may need to run a conversion script on the OSIS XML.
# Placeholder URL -- likely needs manual prep:
Write-Host "  [manual] hebrew-wlc.json requires conversion from OSIS XML"
Write-Host "           Source: https://github.com/openscriptures/morphhb"
Write-Host "           Convert to JSON array of {book, chapter, verse, text, strongs, morphology}"

# --- Greek UGNT (optional) ---
Write-Host "[4/7] Greek (unfoldingWord Greek NT)"
# TODO: update URL -- needs same format as Hebrew
# Source: https://github.com/unfoldingWord/en_ugnt
Write-Host "  [manual] greek-ugnt.json requires conversion from USFM"
Write-Host "           Source: https://github.com/unfoldingWord/en_ugnt"
Write-Host "           Convert to JSON array of {book, chapter, verse, text, strongs, morphology}"

# --- Strong's lexicon (optional) ---
Write-Host "[5/7] Strong's Concordance (Hebrew + Greek)"
# TODO: update URL -- needs {strongs_id, original_word, transliteration, definition, root, semantic_range[]}
# Options:
#   https://github.com/openscriptures/strongs (public domain)
#   https://github.com/samuelfinlayson/strongs-concordance
Write-Host "  [manual] strongs-hebrew.json and strongs-greek.json"
Write-Host "           Source: https://github.com/openscriptures/strongs"
Write-Host "           Convert to JSON array of {strongs_id, original_word, transliteration, definition, root, semantic_range}"

# --- Cross-references (optional) ---
Write-Host "[6/7] Treasury of Scripture Knowledge cross-references"
# TODO: update URL -- needs TSV with header:
#   source_book  source_chapter  source_verse  target_book  target_chapter  target_verse  rel_type
# Options:
#   https://www.openbible.info/labs/cross-references/
#   The openbible.info dataset is tab-separated but needs reformatting.
Write-Host "  [manual] cross-refs.tsv"
Write-Host "           Source: https://www.openbible.info/labs/cross-references/"
Write-Host "           Reformat to TSV: source_book  source_chapter  source_verse  target_book  target_chapter  target_verse  rel_type"

# --- Pericopes (optional) ---
Write-Host "[7/7] Pericope boundaries"
# TODO: update URL or create manually
# Needs JSON array of {title, start_book, start_chapter, start_verse, end_book, end_chapter, end_verse, genre}
Write-Host "  [manual] pericopes.json"
Write-Host "           Create manually or find a dataset of thematic Bible sections"
Write-Host "           Format: [{title, start_book, start_chapter, start_verse, end_book, end_chapter, end_verse, genre}]"

Write-Host ""
Write-Host "=== Summary ==="
Write-Host "Auto-downloaded files (check format matches expected schema):"
Get-ChildItem $dataDir | ForEach-Object { Write-Host "  $($_.Name) ($([math]::Round($_.Length / 1KB, 1)) KB)" }
Write-Host ""
Write-Host "Manual steps remaining:"
Write-Host "  1. Verify kjv.json format: [{book, chapter, verse, text}, ...]"
Write-Host "  2. Convert Hebrew WLC OSIS XML -> hebrew-wlc.json"
Write-Host "  3. Convert Greek UGNT USFM -> greek-ugnt.json"
Write-Host "  4. Convert Strong's data -> strongs-hebrew.json + strongs-greek.json"
Write-Host "  5. Reformat cross-references -> cross-refs.tsv"
Write-Host "  6. Create or find pericope boundaries -> pericopes.json"
Write-Host ""
Write-Host "Once files are ready, run: claw bible-ingest"
