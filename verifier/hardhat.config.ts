import { HardhatUserConfig } from "hardhat/config";
import "@nomicfoundation/hardhat-toolbox";

const INFURA_API_KEY = process.env['INFURA_API_KEY'];
const URL = process.env['URL'];

const config: HardhatUserConfig = {
  solidity: {
    version: "0.8.18",
    settings: {
      optimizer: {
        enabled: true,
        runs: Math.pow(2, 32) - 1
      },
      viaIR: true
    }
  },
  networks: {
    "l1": {
      url: "http://127.0.0.1:18545"
    },
    "custom": {
      url: `${URL}`,
      timeout: 24000,
      accounts: ["5439cff7e3c8d58b312138d3f8fb34ab581e03f7c86c51c46dafa3805e88cc38"]
    }
  },
};

export default config;
