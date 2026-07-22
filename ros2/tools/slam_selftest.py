# Publishes a moving LaserScan + odom TF so slam_toolbox builds a real map.
import math, rclpy
from rclpy.node import Node
from sensor_msgs.msg import LaserScan
from nav_msgs.msg import Odometry
from tf2_ros import TransformBroadcaster
from geometry_msgs.msg import TransformStamped

class Fake(Node):
    def __init__(self):
        super().__init__('fake_robot')
        self.scan_pub = self.create_publisher(LaserScan, '/scan', 10)
        self.br = TransformBroadcaster(self)
        self.t = 0.0
        self.create_timer(0.1, self.tick)
    def tick(self):
        now = self.get_clock().now().to_msg()
        # robot drives in a slow circle
        self.t += 0.1
        x = 1.5 * math.cos(self.t * 0.15)
        y = 1.5 * math.sin(self.t * 0.15)
        yaw = self.t * 0.15 + math.pi/2
        tf = TransformStamped()
        tf.header.stamp = now; tf.header.frame_id = 'odom'; tf.child_frame_id = 'base_footprint'
        tf.transform.translation.x = x; tf.transform.translation.y = y
        tf.transform.rotation.z = math.sin(yaw/2); tf.transform.rotation.w = math.cos(yaw/2)
        self.br.sendTransform(tf)
        # static laser->base
        lt = TransformStamped()
        lt.header.stamp = now; lt.header.frame_id='base_footprint'; lt.child_frame_id='base_scan'
        lt.transform.rotation.w = 1.0
        self.br.sendTransform(lt)
        # a square-room scan (walls at ±3m)
        s = LaserScan()
        s.header.stamp = now; s.header.frame_id = 'base_scan'
        s.angle_min = -math.pi; s.angle_max = math.pi; s.angle_increment = math.pi/180
        s.range_min = 0.1; s.range_max = 10.0
        for i in range(360):
            a = s.angle_min + i*s.angle_increment + yaw
            # distance to a 6x6 box centered at origin, from robot pos
            rng = 10.0
            for wall,(nx,ny,d) in {'r':(1,0,3),'l':(-1,0,3),'t':(0,1,3),'b':(0,-1,3)}.items():
                ca, sa = math.cos(a), math.sin(a)
                denom = nx*ca + ny*sa
                if abs(denom) > 1e-6:
                    t = (d - (nx*x+ny*y))/denom
                    if 0 < t < rng: rng = t
            s.ranges.append(rng)
        self.scan_pub.publish(s)

rclpy.init(); rclpy.spin(Fake())
