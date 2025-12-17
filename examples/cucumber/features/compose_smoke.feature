@compose
Feature: Testing Framework - Compose Runner

  Scenario: Run a compose smoke scenario (tx + DA + liveness)
    Given deployer is "compose"
    And topology has 1 validators and 1 executors
    And wallets total funds is 1000 split across 10 users
    And run duration is 60 seconds
    And transactions rate is 1 per block
    And data availability channel rate is 1 per block and blob rate is 1 per block
    And expect consensus liveness
    When run scenario
    Then scenario should succeed
