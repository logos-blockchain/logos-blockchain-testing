#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

readonly DEFAULT_TAG="test"
readonly DEFAULT_ECR_IMAGE_REPO="public.ecr.aws/r4s5t9y4/logos/logos-blockchain"
readonly DEFAULT_AWS_REGION="us-east-1"
readonly DEFAULT_LOCAL_IMAGE_REPO="logos-blockchain-testing"
readonly DEFAULT_DOCKER_PLATFORM="linux/amd64"
readonly DEFAULT_CIRCUITS_PLATFORM="linux-x86_64"
readonly PUBLIC_ECR_HOST="public.ecr.aws"

# Publishes the testnet image to ECR Public by default.
#
# Env overrides:
#   TAG            - image tag (default: test)
#   ECR_IMAGE_REPO - full repo path without tag (default: public.ecr.aws/r4s5t9y4/logos/logos-blockchain)
#   AWS_REGION     - AWS region for ecr-public login (default: us-east-1)
#
# Legacy (private ECR) overrides:
#   AWS_ACCOUNT_ID - if set, uses private ECR login/push unless ECR_IMAGE_REPO points at public.ecr.aws

TAG="${TAG:-${DEFAULT_TAG}}"
ECR_IMAGE_REPO="${ECR_IMAGE_REPO:-${DEFAULT_ECR_IMAGE_REPO}}"
AWS_REGION="${AWS_REGION:-${DEFAULT_AWS_REGION}}"

LOCAL_IMAGE="${LOCAL_IMAGE:-${DEFAULT_LOCAL_IMAGE_REPO}:${TAG}}"
REMOTE_IMAGE="${ECR_IMAGE_REPO}:${TAG}"

export DOCKER_DEFAULT_PLATFORM="${DEFAULT_DOCKER_PLATFORM}"
export CIRCUITS_PLATFORM="${CIRCUITS_PLATFORM:-${DEFAULT_CIRCUITS_PLATFORM}}"
export IMAGE_TAG="${REMOTE_IMAGE}"

  "${ROOT_DIR}/scripts/build_test_image.sh" --dockerfile "${ROOT_DIR}/testing-framework/assets/stack/Dockerfile.testnet"

if [[ "${ECR_IMAGE_REPO}" == ${PUBLIC_ECR_HOST}/* ]]; then
  aws ecr-public get-login-password --region "${AWS_REGION}" \
    | docker login --username AWS --password-stdin "${PUBLIC_ECR_HOST}"
else
  if [ -z "${AWS_ACCOUNT_ID:-}" ]; then
    echo "ERROR: AWS_ACCOUNT_ID must be set for private ECR pushes (or set ECR_IMAGE_REPO=${PUBLIC_ECR_HOST}/...)" >&2
    exit 1
  fi
  ECR_REGISTRY="${AWS_ACCOUNT_ID}.dkr.ecr.${AWS_REGION}.amazonaws.com"
  aws ecr get-login-password --region "${AWS_REGION}" \
    | docker login --username AWS --password-stdin "${ECR_REGISTRY}"
  docker tag "${REMOTE_IMAGE}" "${ECR_REGISTRY}/${REMOTE_IMAGE#*/}"
  REMOTE_IMAGE="${ECR_REGISTRY}/${REMOTE_IMAGE#*/}"
fi

docker push "${REMOTE_IMAGE}"

echo "${REMOTE_IMAGE}"
