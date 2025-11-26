# Introduction

The Nomos Testing Framework is a purpose-built toolkit for exercising Nomos in
realistic, multi-node environments. It solves the gap between small, isolated
tests and full-system validation by letting teams describe a cluster layout,
drive meaningful traffic, and assert the outcomes in one coherent plan.

It is for protocol engineers, infrastructure operators, and QA teams who need
repeatable confidence that validators, executors, and data-availability
components work together under network and timing constraints.

Multi-node integration testing is required because many Nomos behaviors—block
progress, data availability, liveness under churn—only emerge when several
roles interact over real networking and time. This framework makes those checks
declarative, observable, and portable across environments.
