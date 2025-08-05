#!/usr/bin/env python3
"""
MQTT 測試客戶端

測試 open_moba_backend 的 MQTT 測試介面功能。

用法:
    python mqtt_test_client.py [broker_host] [broker_port]

示例測試命令:
    {"command": "QueryStatus"}
    {"command": "QueryAbilities"}
    {"command": "TestAbility", "data": {"ability_id": "flame_blade", "level": 1}}
    {"command": "TestSummon", "data": {"unit_type": "saika_gunner", "position": [100, 200], "count": 3}}
    {"command": "RunBenchmark", "data": {"test_name": "ability_execution_speed", "iterations": 100}}
    {"command": "QueryMetrics"}
    {"command": "Reset"}
"""

import paho.mqtt.client as mqtt
import json
import time
import sys
import threading
from datetime import datetime

class MqttTestClient:
    def __init__(self, broker_host="localhost", broker_port=1883):
        self.broker_host = broker_host
        self.broker_port = broker_port
        self.client = mqtt.Client()
        self.client.on_connect = self.on_connect
        self.client.on_message = self.on_message
        self.responses = []
        self.response_lock = threading.Lock()
        
    def on_connect(self, client, userdata, flags, rc):
        print(f"已連接到 MQTT Broker: {self.broker_host}:{self.broker_port}")
        client.subscribe("ability_test/response")
        
    def on_message(self, client, userdata, msg):
        try:
            response = json.loads(msg.payload.decode())
            with self.response_lock:
                self.responses.append(response)
            print(f"\n收到回應: {json.dumps(response, indent=2, ensure_ascii=False)}")
        except Exception as e:
            print(f"解析回應失敗: {e}")
            print(f"原始訊息: {msg.payload.decode()}")
    
    def connect(self):
        try:
            self.client.connect(self.broker_host, self.broker_port, 60)
            self.client.loop_start()
            time.sleep(1)  # 等待連接建立
            return True
        except Exception as e:
            print(f"連接失敗: {e}")
            return False
    
    def send_command(self, command_dict):
        """發送測試命令"""
        command_json = json.dumps(command_dict, ensure_ascii=False)
        print(f"\n發送命令: {command_json}")
        
        result = self.client.publish("ability_test/command", command_json)
        if result.rc == mqtt.MQTT_ERR_SUCCESS:
            print("命令已發送")
        else:
            print(f"發送失敗: {result.rc}")
    
    def wait_for_response(self, timeout=5):
        """等待回應"""
        start_time = time.time()
        while time.time() - start_time < timeout:
            with self.response_lock:
                if self.responses:
                    return self.responses.pop(0)
            time.sleep(0.1)
        return None
    
    def run_test_suite(self):
        """運行完整的測試套件"""
        print("=" * 60)
        print("MQTT 測試介面功能測試")
        print("=" * 60)
        
        test_commands = [
            # 1. 查詢系統狀態
            {
                "name": "查詢系統狀態",
                "command": {"command": "QueryStatus"}
            },
            
            # 2. 查詢技能列表
            {
                "name": "查詢技能列表", 
                "command": {"command": "QueryAbilities"}
            },
            
            # 3. 測試技能執行
            {
                "name": "測試火焰刀技能",
                "command": {
                    "command": "TestAbility",
                    "data": {
                        "ability_id": "flame_blade",
                        "level": 1,
                        "target_position": [150, 200]
                    }
                }
            },
            
            # 4. 測試召喚系統
            {
                "name": "測試雜賀鐵炮兵召喚",
                "command": {
                    "command": "TestSummon",
                    "data": {
                        "unit_type": "saika_gunner",
                        "position": [100, 100],
                        "count": 2
                    }
                }
            },
            
            # 5. 運行性能測試
            {
                "name": "運行技能執行速度基準測試",
                "command": {
                    "command": "RunBenchmark",
                    "data": {
                        "test_name": "ability_execution_speed",
                        "iterations": 50
                    }
                }
            },
            
            # 6. 查詢性能統計
            {
                "name": "查詢性能統計",
                "command": {"command": "QueryMetrics"}
            },
            
            # 7. 重置測試環境
            {
                "name": "重置測試環境",
                "command": {"command": "Reset"}
            }
        ]
        
        successful_tests = 0
        total_tests = len(test_commands)
        
        for i, test in enumerate(test_commands, 1):
            print(f"\n[{i}/{total_tests}] {test['name']}")
            print("-" * 40)
            
            self.send_command(test['command'])
            response = self.wait_for_response(timeout=10)
            
            if response:
                if response.get('success', False):
                    print("✅ 測試成功")
                    successful_tests += 1
                else:
                    print("❌ 測試失敗")
                    print(f"錯誤: {response.get('data', {}).get('error', '未知錯誤')}")
            else:
                print("⏰ 測試超時 - 未收到回應")
            
            time.sleep(1)  # 避免過快發送
        
        print("\n" + "=" * 60)
        print(f"測試完成: {successful_tests}/{total_tests} 成功")
        print("=" * 60)
        
        return successful_tests == total_tests

def main():
    broker_host = sys.argv[1] if len(sys.argv) > 1 else "localhost"
    broker_port = int(sys.argv[2]) if len(sys.argv) > 2 else 1883
    
    print(f"MQTT 測試客戶端啟動")
    print(f"目標 Broker: {broker_host}:{broker_port}")
    
    client = MqttTestClient(broker_host, broker_port)
    
    if not client.connect():
        print("無法連接到 MQTT Broker，請確認服務是否運行")
        return 1
    
    try:
        # 等待一段時間確保 open_moba_backend 已經啟動
        print("\n等待 5 秒讓系統完全啟動...")
        time.sleep(5)
        
        # 運行測試套件
        success = client.run_test_suite()
        
        if success:
            print("\n🎉 所有測試通過！")
            return 0
        else:
            print("\n❌ 部分測試失敗")
            return 1
            
    except KeyboardInterrupt:
        print("\n測試被用戶中斷")
        return 1
    finally:
        client.client.loop_stop()
        client.client.disconnect()

if __name__ == "__main__":
    exit(main())