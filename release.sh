#!/bin/bash
# ./release.sh <tag_name> <release_name> <release_body>
# ./release.sh "v1.0.0" "First release" "Initial release of the project."

REPO="evgenyigumnov/cblt"

if [ -z "$GITHUB_TOKEN" ]; then
  echo "Set GITHUB_TOKEN environment variable"
  exit 1
fi


# Параметры релиза
TAG_NAME=$1
RELEASE_NAME=$2
RELEASE_BODY=$3

if [ -z "$TAG_NAME" ] || [ -z "$RELEASE_NAME" ] || [ -z "$RELEASE_BODY" ]; then
  echo "Usage: $0 <tag_name> <release_name> <release_body>"
  exit 1
fi

git tag -a "$TAG_NAME" -m "$RELEASE_NAME"
git push origin "$TAG_NAME"

response=$(curl -s -H "Authorization: token $GITHUB_TOKEN" \
  -H "Content-Type: application/json" \
  -X POST \
  -d "{
    \"tag_name\": \"$TAG_NAME\",
    \"name\": \"$RELEASE_NAME\",
    \"body\": \"$RELEASE_BODY\",
    \"draft\": false,
    \"prerelease\": false
  }" \
  "https://api.github.com/repos/$REPO/releases")

if echo "$response" | grep -q '"url"'; then
  echo "Release created: $TAG_NAME"
else
  echo "Errors: $response"
fi