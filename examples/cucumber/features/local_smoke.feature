@local
Feature: Testing Framework - Local Runner

  Scenario: Run a local smoke scenario (tx + liveness)
    Given deployer is "local"
    And topology has 2 nodes
    And run duration is 60 seconds
    And wallets total funds is 1000000000 split across 50 users
    And transactions rate is 1 per block
    And expect consensus liveness
    When run scenario
    Then scenario should succeed
