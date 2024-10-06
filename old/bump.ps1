param (
    [switch]$local
)

# Define the path to Cargo.toml
$cargoTomlPath = "./Cargo.toml"

# Ensure Cargo.toml exists
if (-Not (Test-Path $cargoTomlPath)) {
    Write-Output "❌ Cargo.toml not found at path: $cargoTomlPath"
    Write-Output "Please ensure the script is run from the root directory of your Rust project."
    exit 1
}

# Read the Cargo.toml content into a variable
$cargoTomlContent = Get-Content -Path $cargoTomlPath -Raw

# Use a regular expression to find the version line
$matched = $cargoTomlContent -match 'version\s*=\s*"(\d+\.\d+\.\d+)"'
if (-Not $matched) {
    Write-Output "❌ Version line not found in Cargo.toml"
    Write-Output "Please ensure the Cargo.toml file contains a valid version line."
    exit 1
}
$versionLine = $matches[1]

# Split the version into major, minor, and patch
$versionParts = $versionLine.Split('.')
$major = $versionParts[0]
$minor = $versionParts[1]
$patch = [int]$versionParts[2]

# Increment the patch version
$patch += 1

# Construct the new version string
$newVersion = "$major.$minor.$patch"

# Replace the old version with the new version in the Cargo.toml content
$newCargoTomlContent = $cargoTomlContent -replace ('version\s*=\s*"' + [regex]::Escape($versionLine) + '"'), ('version = "' + $newVersion + '"')

# Write the new Cargo.toml content back to the file
Set-Content -Path $cargoTomlPath -Value $newCargoTomlContent
Write-Output "✅ Updated version to $newVersion in Cargo.toml"

# Get the current date
$publishDate = Get-Date -Format "yyyy-MM-dd"

# Commit messages with publish date
if ($local) {
    $commitMessage = "🔧 Bump version to $newVersion ($publishDate)"
} else {
    $commitMessage = "🚀 Bump version to $newVersion ($publishDate) and release 📦"
}
$releaseMessage = "Release v$newVersion ($publishDate)"

# Build in release mode and move the binaries to the release folder
$releaseFolder = "./release"
if (Test-Path $releaseFolder) {
    Remove-Item -Recurse -Force $releaseFolder
}
New-Item -ItemType Directory -Path $releaseFolder | Out-Null

 
git add .
git commit -m "$commitMessage"

 

# Push the commit and tag to your repository
Write-Output "🎉 Pushing changes and tags to the repository..."
git push && git push --tags

# Publish the package to crates.io directly
Write-Output "📦 Attempting to publish package to crates.io..."
cargo publish --allow-dirty
if ($LASTEXITCODE -eq 0) {
    Write-Output "✨ Package successfully published to crates.io!"
} else {
    Write-Host "❌ Failed to publish package to crates.io. We did not update the crate." -ForegroundColor Red
}

Write-Output "🎉 Release v$newVersion completed!"
