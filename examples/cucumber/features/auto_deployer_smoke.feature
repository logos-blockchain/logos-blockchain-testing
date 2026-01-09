Feature: Testing Framework - Auto Local/Compose Deployer

  Scenario: Run auto deployer smoke scenario (tx + DA + liveness)
    Given we have a CLI deployer specified
    And topology has 1 validators and 1 executors
    And run duration is 60 seconds
    And wallets total funds is 1000000000 split across 50 users
    And transactions rate is 1 per block
    And data availability channel rate is 1 per block and blob rate is 1 per block
    And expect consensus liveness
    When run scenario
    Then scenario should succeed

  # Note: This test may fail on slow computers
  Scenario: Run auto deployer stress smoke scenario (tx + DA + liveness)
    Given we have a CLI deployer specified
    And topology has 3 validators and 3 executors
    And run duration is 120 seconds
    And wallets total funds is 1000000000 split across 500 users
    And transactions rate is 10 per block
    And data availability channel rate is 1 per block and blob rate is 1 per block
    And expect consensus liveness
    When run scenario
    Then scenario should succeed

  Scenario: Run auto deployer stress smoke scenario no liveness (tx + DA + liveness)
    Given we have a CLI deployer specified
    And topology has 3 validators and 3 executors
    And run duration is 120 seconds
    And wallets total funds is 1000000000 split across 500 users
    And transactions rate is 10 per block
    And data availability channel rate is 1 per block and blob rate is 1 per block
    When run scenario
    Then scenario should succeed
