# Overview

`byovpc-checker` is a simple tool that runs some check against an installed
[Openshift](https://docs.openshift.com/) cluster to verify the setup of subnets
and VPCs is correct.

These checks are meant for [existing
VPCs](https://docs.openshift.com/container-platform/4.15/installing/installing_aws/installing-aws-vpc.html).

## Supported checks

- Verifies tags on subnets.
- Verifies public/private subnets per availability zone.
- Verifies LoadBalancers & subnet association.

## Planned checks

- Allow running in pre-install mode by only running a subset of checks.
- Verify security groups:
  - Check Ingress and compare to LoadBalancer ENI IPs
- Verify ACLs on subnets

## Architecture

The tool is split into 2 distinct phases:

1. Data gathering
2. Running checks

*All* data is gathered upfront, so checks can be pure code, following the
imperative shell, functional core pattern.
